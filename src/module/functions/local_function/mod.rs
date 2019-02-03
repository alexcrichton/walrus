//! Functions defined locally within a wasm module.

mod context;
pub mod display;
mod emit;

use self::context::FunctionContext;
use super::FunctionId;
use crate::dot::Dot;
use crate::emit::IdsToIndices;
use crate::encode::Encoder;
use crate::error::{ErrorKind, Result};
use crate::ir::matcher::{ConstMatcher, Matcher};
use crate::ir::*;
use crate::map::{IdHashMap, IdHashSet};
use crate::module::Module;
use crate::parse::IndicesToIds;
use crate::passes::Used;
use crate::ty::{TypeId, ValType};
use failure::{bail, Fail, ResultExt};
use id_arena::{Arena, Id};
use std::collections::BTreeMap;
use std::fmt;
use std::mem;
use wasmparser::{Operator, OperatorsReader};

/// A function defined locally within the wasm module.
#[derive(Debug)]
pub struct LocalFunction {
    /// This function's type.
    pub ty: TypeId,

    /// The arena that contains this function's expressions.
    pub(crate) exprs: Arena<Expr>,

    args: Vec<LocalId>,

    /// The entry block for this function. Always `Some` after the constructor
    /// returns.
    entry: Option<BlockId>,
    //
    // TODO: provenance: ExprId -> offset in code section of the original
    // instruction. This will be necessary for preserving debug info.
}

impl LocalFunction {
    /// Creates a new definition of a local function from its components
    pub(crate) fn new(
        ty: TypeId,
        args: Vec<LocalId>,
        exprs: Arena<Expr>,
        entry: BlockId,
    ) -> LocalFunction {
        LocalFunction {
            ty,
            args,
            entry: Some(entry),
            exprs,
        }
    }

    /// Construct a new `LocalFunction`.
    ///
    /// Validates the given function body and constructs the `Expr` IR at the
    /// same time.
    pub(crate) fn parse(
        module: &Module,
        indices: &IndicesToIds,
        id: FunctionId,
        ty: TypeId,
        args: Vec<LocalId>,
        body: wasmparser::OperatorsReader,
    ) -> Result<LocalFunction> {
        let mut func = LocalFunction {
            ty,
            exprs: Arena::new(),
            args,
            entry: None,
        };

        let result: Vec<_> = module.types.get(ty).results().iter().cloned().collect();
        let result = result.into_boxed_slice();
        let result_len = result.len();

        let operands = &mut context::OperandStack::new();
        let controls = &mut context::ControlStack::new();

        let mut ctx = FunctionContext::new(module, indices, id, &mut func, operands, controls);

        let entry = ctx.push_control(BlockKind::FunctionEntry, result.clone(), result);
        ctx.func.entry = Some(entry);
        validate_expression(&mut ctx, body)?;

        debug_assert_eq!(ctx.operands.len(), result_len);
        debug_assert!(ctx.controls.is_empty());

        Ok(func)
    }

    /// Allocate a new expression into this local function's arena
    pub(crate) fn alloc<T>(&mut self, val: T) -> T::Id
    where
        T: Ast,
    {
        let id = self.exprs.alloc(val.into());
        T::new_id(id)
    }

    /// Get the id of this function's entry block.
    pub fn entry_block(&self) -> BlockId {
        self.entry.unwrap()
    }

    /// Get the block associated with the given id.
    pub fn block(&self, block: BlockId) -> &Block {
        self.exprs[block.into()].unwrap_block()
    }

    /// Get the block associated with the given id.
    pub fn block_mut(&mut self, block: BlockId) -> &mut Block {
        self.exprs[block.into()].unwrap_block_mut()
    }

    /// Get the size of this function, in number of expressions.
    pub fn size(&self) -> u64 {
        struct SizeVisitor<'a> {
            func: &'a LocalFunction,
            exprs: u64,
        }

        impl<'expr> Visitor<'expr> for SizeVisitor<'expr> {
            fn local_function(&self) -> &'expr LocalFunction {
                self.func
            }

            fn visit_expr(&mut self, e: &'expr Expr) {
                self.exprs += 1;
                e.visit(self);
            }
        }

        let mut v = SizeVisitor {
            func: self,
            exprs: 0,
        };
        self.entry_block().visit(&mut v);
        v.exprs
    }

    /// Is this function's body a [constant
    /// expression](https://webassembly.github.io/spec/core/valid/instructions.html#constant-expressions)?
    pub fn is_const(&self) -> bool {
        let entry = match &self.exprs[self.entry_block().into()] {
            Expr::Block(b) => b,
            _ => unreachable!(),
        };
        let matcher = ConstMatcher::new();
        entry
            .exprs
            .iter()
            .all(|e| matcher.is_match(self, &self.exprs[*e]))
    }

    /// Emit this function's compact locals declarations.
    pub(crate) fn emit_locals(
        &self,
        id: FunctionId,
        module: &Module,
        used: &Used,
        encoder: &mut Encoder,
    ) -> IdHashMap<Local, u32> {
        let mut used_locals = Vec::new();
        if let Some(locals) = used.locals.get(&id) {
            used_locals = locals.iter().cloned().collect();
            // Sort to ensure we assign local indexes deterministically, and
            // everything is distinct so we can use a faster unstable sort.
            used_locals.sort_unstable();
        }

        // NB: Use `BTreeMap` to make compilation deterministic by emitting
        // types in the same order
        let mut ty_to_locals = BTreeMap::new();
        let args = self.args.iter().cloned().collect::<IdHashSet<_>>();

        // Partition all locals by their type as we'll create at most one entry
        // for each type. Skip all arguments to the function because they're
        // handled separately.
        for local in used_locals.iter() {
            if !args.contains(local) {
                let ty = module.locals.get(*local).ty();
                ty_to_locals.entry(ty).or_insert_with(Vec::new).push(*local);
            }
        }

        let mut local_map = IdHashMap::default();
        local_map.reserve(used_locals.len());

        // Allocate an index to all the function arguments, as these are all
        // unconditionally used and are implicit locals in wasm.
        let mut idx = 0;
        for &arg in self.args.iter() {
            local_map.insert(arg, idx);
            idx += 1;
        }

        // Assign an index to all remaining locals
        for (_, locals) in ty_to_locals.iter() {
            for l in locals {
                local_map.insert(*l, idx);
                idx += 1;
            }
        }

        // Use our type map to emit a compact representation of all locals now
        encoder.usize(ty_to_locals.len());
        for (ty, locals) in ty_to_locals.iter() {
            encoder.usize(locals.len());
            ty.emit(encoder);
        }

        local_map
    }

    /// Emit this function's instruction sequence.
    pub(crate) fn emit_instructions(
        &self,
        indices: &IdsToIndices,
        local_indices: &IdHashMap<Local, u32>,
        dst: &mut Encoder,
    ) {
        emit::run(self, indices, local_indices, dst)
    }
}

impl fmt::Display for LocalFunction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::display::DisplayIr;
        let mut dst = String::new();
        self.display_ir(&mut dst, &(), 0);
        dst.fmt(f)
    }
}

impl Dot for LocalFunction {
    fn dot(&self, out: &mut String) {
        out.push_str("digraph {\n");
        out.push_str("rankdir=LR;\n");

        let v = &mut DotExpr {
            out,
            func: self,
            id: self.entry_block().into(),
            needs_close: false,
        };
        v.out.push_str("subgraph unreachable {\n");
        self.entry_block().visit(v);
        v.close_previous();
        v.out.push_str("}\n");
        out.push_str("}\n");
    }
}

pub(crate) struct DotExpr<'a, 'b> {
    pub(crate) out: &'a mut String,
    pub(crate) func: &'b LocalFunction,
    id: ExprId,
    needs_close: bool,
}

impl DotExpr<'_, '_> {
    pub(crate) fn expr_id(&mut self, id: ExprId) {
        self.close_previous();
        let prev = mem::replace(&mut self.id, id);
        id.dot(self.out);
        self.out.push_str(
            " [label=<<table cellborder=\"0\" border=\"0\"><tr><td><font face=\"monospace\">",
        );
        self.needs_close = true;
        id.visit(self);
        self.close_previous();
        self.id = prev;
    }

    pub(crate) fn id<T>(&mut self, id: Id<T>) {
        self.out.push_str(" ");
        self.out.push_str(&id.index().to_string());
    }

    fn close_previous(&mut self) {
        if self.needs_close {
            self.out.push_str("</font></td></tr></table>>];\n")
        }
        self.needs_close = false;
    }

    pub(crate) fn edge<E, S>(&mut self, to: E, label: S)
    where
        E: Into<ExprId>,
        S: AsRef<str>,
    {
        self._edge(to.into(), label.as_ref())
    }

    fn _edge(&mut self, to: ExprId, label: &str) {
        self.close_previous();
        self.id.dot(self.out);
        self.out.push_str(" -> ");
        to.dot(self.out);
        self.out.push_str(&format!(" [label=\"{}\"];\n", label));
    }
}

fn validate_instruction_sequence_until_end(
    ctx: &mut FunctionContext,
    ops: &mut OperatorsReader,
) -> Result<()> {
    validate_instruction_sequence_until(ctx, ops, |i| match i {
        Operator::End => true,
        _ => false,
    })
}

fn validate_instruction_sequence_until(
    ctx: &mut FunctionContext,
    ops: &mut OperatorsReader,
    mut until: impl FnMut(&Operator) -> bool,
) -> Result<()> {
    loop {
        let inst = ops.read()?;
        if until(&inst) {
            return Ok(());
        }
        validate_instruction(ctx, inst, ops)?;
    }
}

fn validate_expression(ctx: &mut FunctionContext, mut ops: OperatorsReader) -> Result<BlockId> {
    validate_instruction_sequence_until_end(ctx, &mut ops)?;
    let block = validate_end(ctx)?;
    ops.ensure_end()?;
    Ok(block)
}

fn validate_end(ctx: &mut FunctionContext) -> Result<BlockId> {
    let (results, block) = ctx.pop_control()?;
    ctx.push_operands(&results, block.into());
    Ok(block)
}

fn validate_instruction(
    ctx: &mut FunctionContext,
    inst: Operator,
    ops: &mut OperatorsReader,
) -> Result<()> {
    use ExtendedLoad::*;
    use ValType::*;

    let const_ = |ctx: &mut FunctionContext, ty, value| {
        let expr = ctx.func.alloc(Const { value });
        ctx.push_operand(Some(ty), expr);
    };

    let one_op = |ctx: &mut FunctionContext, input, output, op| -> Result<()> {
        let (_, expr) = ctx.pop_operand_expected(Some(input))?;
        let expr = ctx.func.alloc(Unop { op, expr });
        ctx.push_operand(Some(output), expr);
        Ok(())
    };
    let two_ops = |ctx: &mut FunctionContext, input, output, op| -> Result<()> {
        let (_, rhs) = ctx.pop_operand_expected(Some(input))?;
        let (_, lhs) = ctx.pop_operand_expected(Some(input))?;
        let expr = ctx.func.alloc(Binop { op, lhs, rhs });
        ctx.push_operand(Some(output), expr);
        Ok(())
    };

    let binop = |ctx, ty, op| two_ops(ctx, ty, ty, op);
    let unop = |ctx, ty, op| one_op(ctx, ty, ty, op);
    let testop = |ctx, ty, op| one_op(ctx, ty, I32, op);
    let relop = |ctx, ty, op| two_ops(ctx, ty, I32, op);

    let mem_arg = |arg: &wasmparser::MemoryImmediate| -> Result<MemArg> {
        if arg.flags >= 32 {
            failure::bail!("invalid alignment");
        }
        Ok(MemArg {
            align: 1 << (arg.flags as i32),
            offset: arg.offset,
        })
    };

    let load = |ctx: &mut FunctionContext, arg, ty, kind| -> Result<()> {
        let (_, address) = ctx.pop_operand_expected(Some(I32))?;
        let memory = ctx.indices.get_memory(0)?;
        let arg = mem_arg(&arg)?;
        let expr = ctx.func.alloc(Load {
            arg,
            kind,
            address,
            memory,
        });
        ctx.push_operand(Some(ty), expr);
        Ok(())
    };

    let store = |ctx: &mut FunctionContext, arg, ty, kind| -> Result<()> {
        let (_, value) = ctx.pop_operand_expected(Some(ty))?;
        let (_, address) = ctx.pop_operand_expected(Some(I32))?;
        let memory = ctx.indices.get_memory(0)?;
        let arg = mem_arg(&arg)?;
        let expr = ctx.func.alloc(Store {
            arg,
            kind,
            address,
            memory,
            value,
        });
        ctx.add_to_current_frame_block(expr);
        Ok(())
    };

    let atomicrmw = |ctx: &mut FunctionContext, arg, ty, op, width| -> Result<()> {
        let (_, value) = ctx.pop_operand_expected(Some(ty))?;
        let (_, address) = ctx.pop_operand_expected(Some(I32))?;
        let memory = ctx.indices.get_memory(0)?;
        let arg = mem_arg(&arg)?;
        let expr = ctx.func.alloc(AtomicRmw {
            arg,
            address,
            memory,
            value,
            op,
            width,
        });
        ctx.push_operand(Some(ty), expr);
        Ok(())
    };

    let cmpxchg = |ctx: &mut FunctionContext, arg, ty, width| -> Result<()> {
        let (_, replacement) = ctx.pop_operand_expected(Some(ty))?;
        let (_, expected) = ctx.pop_operand_expected(Some(ty))?;
        let (_, address) = ctx.pop_operand_expected(Some(I32))?;
        let memory = ctx.indices.get_memory(0)?;
        let arg = mem_arg(&arg)?;
        let expr = ctx.func.alloc(Cmpxchg {
            arg,
            address,
            memory,
            expected,
            width,
            replacement,
        });
        ctx.push_operand(Some(ty), expr);
        Ok(())
    };

    match inst {
        Operator::Call { function_index } => {
            let func = ctx
                .indices
                .get_func(function_index)
                .context("invalid call")?;
            let ty_id = ctx.module.funcs.get(func).ty();
            let fun_ty = ctx.module.types.get(ty_id);
            let mut args = ctx.pop_operands(fun_ty.params())?.into_boxed_slice();
            args.reverse();
            let expr = ctx.func.alloc(Call { func, args });
            ctx.push_operands(fun_ty.results(), expr.into());
        }
        Operator::CallIndirect { index, table_index } => {
            let type_id = ctx
                .indices
                .get_type(index)
                .context("invalid call_indirect")?;
            let ty = ctx.module.types.get(type_id);
            let table = ctx
                .indices
                .get_table(table_index)
                .context("invalid call_indirect")?;
            let (_, func) = ctx.pop_operand_expected(Some(I32))?;
            let mut args = ctx.pop_operands(ty.params())?.into_boxed_slice();
            args.reverse();
            let expr = ctx.func.alloc(CallIndirect {
                table,
                ty: type_id,
                func,
                args,
            });
            ctx.push_operands(ty.results(), expr.into());
        }
        Operator::GetLocal { local_index } => {
            let local = ctx.indices.get_local(ctx.func_id, local_index)?;
            let ty = ctx.module.locals.get(local).ty();
            let expr = ctx.func.alloc(LocalGet { local });
            ctx.push_operand(Some(ty), expr);
        }
        Operator::SetLocal { local_index } => {
            let local = ctx.indices.get_local(ctx.func_id, local_index)?;
            let ty = ctx.module.locals.get(local).ty();
            let (_, value) = ctx.pop_operand_expected(Some(ty))?;
            let expr = ctx.func.alloc(LocalSet { local, value });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::TeeLocal { local_index } => {
            let local = ctx.indices.get_local(ctx.func_id, local_index)?;
            let ty = ctx.module.locals.get(local).ty();
            let (_, value) = ctx.pop_operand_expected(Some(ty))?;
            let expr = ctx.func.alloc(LocalTee { local, value });
            ctx.push_operand(Some(ty), expr);
        }
        Operator::GetGlobal { global_index } => {
            let global = ctx
                .indices
                .get_global(global_index)
                .context("invalid global.get")?;
            let ty = ctx.module.globals.get(global).ty;
            let expr = ctx.func.alloc(GlobalGet { global });
            ctx.push_operand(Some(ty), expr);
        }
        Operator::SetGlobal { global_index } => {
            let global = ctx
                .indices
                .get_global(global_index)
                .context("invalid global.set")?;
            let ty = ctx.module.globals.get(global).ty;
            let (_, value) = ctx.pop_operand_expected(Some(ty))?;
            let expr = ctx.func.alloc(GlobalSet { global, value });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::I32Const { value } => const_(ctx, I32, Value::I32(value)),
        Operator::I64Const { value } => const_(ctx, I64, Value::I64(value)),
        Operator::F32Const { value } => {
            const_(ctx, F32, Value::F32(f32::from_bits(value.bits())));
        }
        Operator::F64Const { value } => {
            const_(ctx, F64, Value::F64(f64::from_bits(value.bits())));
        }
        Operator::V128Const { value } => {
            let n = value.bytes();
            let val = ((n[0] as u128) << 0)
                | ((n[1] as u128) << 8)
                | ((n[2] as u128) << 16)
                | ((n[3] as u128) << 24)
                | ((n[4] as u128) << 32)
                | ((n[5] as u128) << 40)
                | ((n[6] as u128) << 48)
                | ((n[7] as u128) << 56)
                | ((n[8] as u128) << 64)
                | ((n[9] as u128) << 72)
                | ((n[10] as u128) << 80)
                | ((n[11] as u128) << 88)
                | ((n[12] as u128) << 96)
                | ((n[13] as u128) << 104)
                | ((n[14] as u128) << 112)
                | ((n[15] as u128) << 120);
            const_(ctx, V128, Value::V128(val));
        }
        Operator::I32Eqz => testop(ctx, I32, UnaryOp::I32Eqz)?,
        Operator::I32Eq => relop(ctx, I32, BinaryOp::I32Eq)?,
        Operator::I32Ne => relop(ctx, I32, BinaryOp::I32Ne)?,
        Operator::I32LtS => relop(ctx, I32, BinaryOp::I32LtS)?,
        Operator::I32LtU => relop(ctx, I32, BinaryOp::I32LtU)?,
        Operator::I32GtS => relop(ctx, I32, BinaryOp::I32GtS)?,
        Operator::I32GtU => relop(ctx, I32, BinaryOp::I32GtU)?,
        Operator::I32LeS => relop(ctx, I32, BinaryOp::I32LeS)?,
        Operator::I32LeU => relop(ctx, I32, BinaryOp::I32LeU)?,
        Operator::I32GeS => relop(ctx, I32, BinaryOp::I32GeS)?,
        Operator::I32GeU => relop(ctx, I32, BinaryOp::I32GeU)?,

        Operator::I64Eqz => testop(ctx, I64, UnaryOp::I64Eqz)?,
        Operator::I64Eq => relop(ctx, I64, BinaryOp::I64Eq)?,
        Operator::I64Ne => relop(ctx, I64, BinaryOp::I64Ne)?,
        Operator::I64LtS => relop(ctx, I64, BinaryOp::I64LtS)?,
        Operator::I64LtU => relop(ctx, I64, BinaryOp::I64LtU)?,
        Operator::I64GtS => relop(ctx, I64, BinaryOp::I64GtS)?,
        Operator::I64GtU => relop(ctx, I64, BinaryOp::I64GtU)?,
        Operator::I64LeS => relop(ctx, I64, BinaryOp::I64LeS)?,
        Operator::I64LeU => relop(ctx, I64, BinaryOp::I64LeU)?,
        Operator::I64GeS => relop(ctx, I64, BinaryOp::I64GeS)?,
        Operator::I64GeU => relop(ctx, I64, BinaryOp::I64GeU)?,

        Operator::F32Eq => relop(ctx, F32, BinaryOp::F32Eq)?,
        Operator::F32Ne => relop(ctx, F32, BinaryOp::F32Ne)?,
        Operator::F32Lt => relop(ctx, F32, BinaryOp::F32Lt)?,
        Operator::F32Gt => relop(ctx, F32, BinaryOp::F32Gt)?,
        Operator::F32Le => relop(ctx, F32, BinaryOp::F32Le)?,
        Operator::F32Ge => relop(ctx, F32, BinaryOp::F32Ge)?,

        Operator::F64Eq => relop(ctx, F64, BinaryOp::F64Eq)?,
        Operator::F64Ne => relop(ctx, F64, BinaryOp::F64Ne)?,
        Operator::F64Lt => relop(ctx, F64, BinaryOp::F64Lt)?,
        Operator::F64Gt => relop(ctx, F64, BinaryOp::F64Gt)?,
        Operator::F64Le => relop(ctx, F64, BinaryOp::F64Le)?,
        Operator::F64Ge => relop(ctx, F64, BinaryOp::F64Ge)?,

        Operator::I32Clz => unop(ctx, I32, UnaryOp::I32Clz)?,
        Operator::I32Ctz => unop(ctx, I32, UnaryOp::I32Ctz)?,
        Operator::I32Popcnt => unop(ctx, I32, UnaryOp::I32Popcnt)?,
        Operator::I32Add => binop(ctx, I32, BinaryOp::I32Add)?,
        Operator::I32Sub => binop(ctx, I32, BinaryOp::I32Sub)?,
        Operator::I32Mul => binop(ctx, I32, BinaryOp::I32Mul)?,
        Operator::I32DivS => binop(ctx, I32, BinaryOp::I32DivS)?,
        Operator::I32DivU => binop(ctx, I32, BinaryOp::I32DivU)?,
        Operator::I32RemS => binop(ctx, I32, BinaryOp::I32RemS)?,
        Operator::I32RemU => binop(ctx, I32, BinaryOp::I32RemU)?,
        Operator::I32And => binop(ctx, I32, BinaryOp::I32And)?,
        Operator::I32Or => binop(ctx, I32, BinaryOp::I32Or)?,
        Operator::I32Xor => binop(ctx, I32, BinaryOp::I32Xor)?,
        Operator::I32Shl => binop(ctx, I32, BinaryOp::I32Shl)?,
        Operator::I32ShrS => binop(ctx, I32, BinaryOp::I32ShrS)?,
        Operator::I32ShrU => binop(ctx, I32, BinaryOp::I32ShrU)?,
        Operator::I32Rotl => binop(ctx, I32, BinaryOp::I32Rotl)?,
        Operator::I32Rotr => binop(ctx, I32, BinaryOp::I32Rotr)?,

        Operator::I64Clz => unop(ctx, I64, UnaryOp::I64Clz)?,
        Operator::I64Ctz => unop(ctx, I64, UnaryOp::I64Ctz)?,
        Operator::I64Popcnt => unop(ctx, I64, UnaryOp::I64Popcnt)?,
        Operator::I64Add => binop(ctx, I64, BinaryOp::I64Add)?,
        Operator::I64Sub => binop(ctx, I64, BinaryOp::I64Sub)?,
        Operator::I64Mul => binop(ctx, I64, BinaryOp::I64Mul)?,
        Operator::I64DivS => binop(ctx, I64, BinaryOp::I64DivS)?,
        Operator::I64DivU => binop(ctx, I64, BinaryOp::I64DivU)?,
        Operator::I64RemS => binop(ctx, I64, BinaryOp::I64RemS)?,
        Operator::I64RemU => binop(ctx, I64, BinaryOp::I64RemU)?,
        Operator::I64And => binop(ctx, I64, BinaryOp::I64And)?,
        Operator::I64Or => binop(ctx, I64, BinaryOp::I64Or)?,
        Operator::I64Xor => binop(ctx, I64, BinaryOp::I64Xor)?,
        Operator::I64Shl => binop(ctx, I64, BinaryOp::I64Shl)?,
        Operator::I64ShrS => binop(ctx, I64, BinaryOp::I64ShrS)?,
        Operator::I64ShrU => binop(ctx, I64, BinaryOp::I64ShrU)?,
        Operator::I64Rotl => binop(ctx, I64, BinaryOp::I64Rotl)?,
        Operator::I64Rotr => binop(ctx, I64, BinaryOp::I64Rotr)?,

        Operator::F32Abs => unop(ctx, F32, UnaryOp::F32Abs)?,
        Operator::F32Neg => unop(ctx, F32, UnaryOp::F32Neg)?,
        Operator::F32Ceil => unop(ctx, F32, UnaryOp::F32Ceil)?,
        Operator::F32Floor => unop(ctx, F32, UnaryOp::F32Floor)?,
        Operator::F32Trunc => unop(ctx, F32, UnaryOp::F32Trunc)?,
        Operator::F32Nearest => unop(ctx, F32, UnaryOp::F32Nearest)?,
        Operator::F32Sqrt => unop(ctx, F32, UnaryOp::F32Sqrt)?,
        Operator::F32Add => binop(ctx, F32, BinaryOp::F32Add)?,
        Operator::F32Sub => binop(ctx, F32, BinaryOp::F32Sub)?,
        Operator::F32Mul => binop(ctx, F32, BinaryOp::F32Mul)?,
        Operator::F32Div => binop(ctx, F32, BinaryOp::F32Div)?,
        Operator::F32Min => binop(ctx, F32, BinaryOp::F32Min)?,
        Operator::F32Max => binop(ctx, F32, BinaryOp::F32Max)?,
        Operator::F32Copysign => binop(ctx, F32, BinaryOp::F32Copysign)?,

        Operator::F64Abs => unop(ctx, F64, UnaryOp::F64Abs)?,
        Operator::F64Neg => unop(ctx, F64, UnaryOp::F64Neg)?,
        Operator::F64Ceil => unop(ctx, F64, UnaryOp::F64Ceil)?,
        Operator::F64Floor => unop(ctx, F64, UnaryOp::F64Floor)?,
        Operator::F64Trunc => unop(ctx, F64, UnaryOp::F64Trunc)?,
        Operator::F64Nearest => unop(ctx, F64, UnaryOp::F64Nearest)?,
        Operator::F64Sqrt => unop(ctx, F64, UnaryOp::F64Sqrt)?,
        Operator::F64Add => binop(ctx, F64, BinaryOp::F64Add)?,
        Operator::F64Sub => binop(ctx, F64, BinaryOp::F64Sub)?,
        Operator::F64Mul => binop(ctx, F64, BinaryOp::F64Mul)?,
        Operator::F64Div => binop(ctx, F64, BinaryOp::F64Div)?,
        Operator::F64Min => binop(ctx, F64, BinaryOp::F64Min)?,
        Operator::F64Max => binop(ctx, F64, BinaryOp::F64Max)?,
        Operator::F64Copysign => binop(ctx, F64, BinaryOp::F64Copysign)?,

        Operator::I32WrapI64 => one_op(ctx, I64, I32, UnaryOp::I32WrapI64)?,
        Operator::I32TruncSF32 => one_op(ctx, F32, I32, UnaryOp::I32TruncSF32)?,
        Operator::I32TruncUF32 => one_op(ctx, F32, I32, UnaryOp::I32TruncUF32)?,
        Operator::I32TruncSF64 => one_op(ctx, F64, I32, UnaryOp::I32TruncSF64)?,
        Operator::I32TruncUF64 => one_op(ctx, F64, I32, UnaryOp::I32TruncUF64)?,

        Operator::I64ExtendSI32 => one_op(ctx, I32, I64, UnaryOp::I64ExtendSI32)?,
        Operator::I64ExtendUI32 => one_op(ctx, I32, I64, UnaryOp::I64ExtendUI32)?,
        Operator::I64TruncSF32 => one_op(ctx, F32, I64, UnaryOp::I64TruncSF32)?,
        Operator::I64TruncUF32 => one_op(ctx, F32, I64, UnaryOp::I64TruncUF32)?,
        Operator::I64TruncSF64 => one_op(ctx, F64, I64, UnaryOp::I64TruncSF64)?,
        Operator::I64TruncUF64 => one_op(ctx, F64, I64, UnaryOp::I64TruncUF64)?,

        Operator::F32ConvertSI32 => one_op(ctx, I32, F32, UnaryOp::F32ConvertSI32)?,
        Operator::F32ConvertUI32 => one_op(ctx, I32, F32, UnaryOp::F32ConvertUI32)?,
        Operator::F32ConvertSI64 => one_op(ctx, I64, F32, UnaryOp::F32ConvertSI64)?,
        Operator::F32ConvertUI64 => one_op(ctx, I64, F32, UnaryOp::F32ConvertUI64)?,
        Operator::F32DemoteF64 => one_op(ctx, F64, F32, UnaryOp::F32DemoteF64)?,

        Operator::F64ConvertSI32 => one_op(ctx, I32, F64, UnaryOp::F64ConvertSI32)?,
        Operator::F64ConvertUI32 => one_op(ctx, I32, F64, UnaryOp::F64ConvertUI32)?,
        Operator::F64ConvertSI64 => one_op(ctx, I64, F64, UnaryOp::F64ConvertSI64)?,
        Operator::F64ConvertUI64 => one_op(ctx, I64, F64, UnaryOp::F64ConvertUI64)?,
        Operator::F64PromoteF32 => one_op(ctx, F32, F64, UnaryOp::F64PromoteF32)?,

        Operator::I32ReinterpretF32 => one_op(ctx, F32, I32, UnaryOp::I32ReinterpretF32)?,
        Operator::I64ReinterpretF64 => one_op(ctx, F64, I64, UnaryOp::I64ReinterpretF64)?,
        Operator::F32ReinterpretI32 => one_op(ctx, I32, F32, UnaryOp::F32ReinterpretI32)?,
        Operator::F64ReinterpretI64 => one_op(ctx, I64, F64, UnaryOp::F64ReinterpretI64)?,

        Operator::I32Extend8S => one_op(ctx, I32, I32, UnaryOp::I32Extend8S)?,
        Operator::I32Extend16S => one_op(ctx, I32, I32, UnaryOp::I32Extend16S)?,
        Operator::I64Extend8S => one_op(ctx, I64, I64, UnaryOp::I64Extend8S)?,
        Operator::I64Extend16S => one_op(ctx, I64, I64, UnaryOp::I64Extend16S)?,
        Operator::I64Extend32S => one_op(ctx, I64, I64, UnaryOp::I64Extend32S)?,

        Operator::Drop => {
            let (_, expr) = ctx.pop_operand()?;
            let expr = ctx.func.alloc(Drop { expr });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::Select => {
            let (_, condition) = ctx.pop_operand_expected(Some(I32))?;
            let (t1, consequent) = ctx.pop_operand()?;
            let (t2, alternative) = ctx.pop_operand_expected(t1)?;
            let expr = ctx.func.alloc(Select {
                condition,
                consequent,
                alternative,
            });
            ctx.push_operand(t2, expr);
        }
        Operator::Return => {
            let fn_ty = ctx.module.funcs.get(ctx.func_id).ty();
            let expected = ctx.module.types.get(fn_ty).results();
            let values = ctx.pop_operands(expected)?.into_boxed_slice();
            let expr = ctx.func.alloc(Return { values });
            ctx.unreachable(expr);
        }
        Operator::Unreachable => {
            let expr = ctx.func.alloc(Unreachable {});
            ctx.unreachable(expr);
        }
        Operator::Block { ty } => {
            let results = ValType::from_block_ty(ty)?;
            ctx.push_control(BlockKind::Block, results.clone(), results);
            validate_instruction_sequence_until_end(ctx, ops)?;
            validate_end(ctx)?;
        }
        Operator::Loop { ty } => {
            let t = ValType::from_block_ty(ty)?;
            ctx.push_control(BlockKind::Loop, vec![].into_boxed_slice(), t);
            validate_instruction_sequence_until_end(ctx, ops)?;
            validate_end(ctx)?;
        }
        Operator::If { ty } => {
            let ty = ValType::from_block_ty(ty)?;
            let (_, condition) = ctx.pop_operand_expected(Some(I32))?;

            let consequent = ctx.push_control(BlockKind::IfElse, ty.clone(), ty.clone());

            let mut else_found = false;
            validate_instruction_sequence_until(ctx, ops, |i| match i {
                Operator::Else => {
                    else_found = true;
                    true
                }
                Operator::End => true,
                _ => false,
            })?;
            let (results, _block) = ctx.pop_control()?;

            let alternative = ctx.push_control(BlockKind::IfElse, results.clone(), results);

            if else_found {
                validate_instruction_sequence_until_end(ctx, ops)?;
            }
            let (results, _block) = ctx.pop_control()?;

            let expr = ctx.func.alloc(IfElse {
                condition,
                consequent,
                alternative,
            });
            ctx.push_operands(&results, expr.into());
        }
        Operator::End => {
            return Err(ErrorKind::InvalidWasm
                .context("unexpected `end` instruction")
                .into());
        }
        Operator::Else => {
            return Err(ErrorKind::InvalidWasm
                .context("`else` without a leading `if`")
                .into());
        }
        Operator::Br { relative_depth } => {
            let n = relative_depth as usize;
            let expected = ctx.control(n)?.label_types.clone();
            let args = ctx.pop_operands(&expected)?.into_boxed_slice();

            let to_block = ctx.control(n)?.block;
            let expr = ctx.func.alloc(Br {
                block: to_block,
                args,
            });
            ctx.unreachable(expr);
        }
        Operator::BrIf { relative_depth } => {
            let n = relative_depth as usize;
            let (_, condition) = ctx.pop_operand_expected(Some(I32))?;

            let expected = ctx.control(n)?.label_types.clone();
            let args = ctx.pop_operands(&expected)?.into_boxed_slice();

            let to_block = ctx.control(n)?.block;
            let expr = ctx.func.alloc(BrIf {
                condition,
                block: to_block,
                args,
            });
            ctx.push_operands(&expected, expr.into());
        }
        Operator::BrTable { table } => {
            let len = table.len();
            let mut blocks = Vec::with_capacity(len);
            let mut label_types = None;
            let mut iter = table.into_iter();
            let mut next = || {
                let n = match iter.next() {
                    Some(n) => n,
                    None => bail!("malformed `br_table"),
                };
                let control = ctx.control(n as usize)?;
                match label_types {
                    None => label_types = Some(&control.label_types),
                    Some(n) => {
                        if n != &control.label_types {
                            bail!("br_table jump with non-uniform label types")
                        }
                    }
                }
                Ok(control.block)
            };
            for _ in 0..len {
                blocks.push(next()?);
            }
            let default = next()?;
            if iter.next().is_some() {
                bail!("malformed `br_table`");
            }

            let blocks = blocks.into_boxed_slice();
            let expected = label_types.unwrap().clone();
            let (_, which) = ctx.pop_operand_expected(Some(I32))?;

            let args = ctx.pop_operands(&expected)?.into_boxed_slice();
            let expr = ctx.func.alloc(BrTable {
                which,
                blocks,
                default,
                args,
            });

            ctx.unreachable(expr);
        }

        Operator::MemorySize { reserved } => {
            if reserved != 0 {
                bail!("reserved byte isn't zero");
            }
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(MemorySize { memory });
            ctx.push_operand(Some(I32), expr);
        }
        Operator::MemoryGrow { reserved } => {
            if reserved != 0 {
                bail!("reserved byte isn't zero");
            }
            let (_, pages) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(MemoryGrow { memory, pages });
            ctx.push_operand(Some(I32), expr);
        }
        Operator::MemoryInit { segment } => {
            let (_, len) = ctx.pop_operand_expected(Some(I32))?;
            let (_, data_offset) = ctx.pop_operand_expected(Some(I32))?;
            let (_, memory_offset) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let data = ctx.indices.get_data(segment)?;
            let expr = ctx.func.alloc(MemoryInit {
                len,
                data_offset,
                memory_offset,
                memory,
                data,
            });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::DataDrop { segment } => {
            let data = ctx.indices.get_data(segment)?;
            let expr = ctx.func.alloc(DataDrop { data });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::MemoryCopy => {
            let (_, len) = ctx.pop_operand_expected(Some(I32))?;
            let (_, src_offset) = ctx.pop_operand_expected(Some(I32))?;
            let (_, dst_offset) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(MemoryCopy {
                len,
                src_offset,
                dst_offset,
                src: memory,
                dst: memory,
            });
            ctx.add_to_current_frame_block(expr);
        }
        Operator::MemoryFill => {
            let (_, len) = ctx.pop_operand_expected(Some(I32))?;
            let (_, value) = ctx.pop_operand_expected(Some(I32))?;
            let (_, offset) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(MemoryFill {
                len,
                offset,
                value,
                memory,
            });
            ctx.add_to_current_frame_block(expr);
        }

        Operator::Nop => {}

        Operator::I32Load { memarg } => load(ctx, memarg, I32, LoadKind::I32 { atomic: false })?,
        Operator::I64Load { memarg } => load(ctx, memarg, I64, LoadKind::I64 { atomic: false })?,
        Operator::F32Load { memarg } => load(ctx, memarg, F32, LoadKind::F32)?,
        Operator::F64Load { memarg } => load(ctx, memarg, F64, LoadKind::F64)?,
        Operator::V128Load { memarg } => load(ctx, memarg, V128, LoadKind::V128)?,
        Operator::I32Load8S { memarg } => {
            load(ctx, memarg, I32, LoadKind::I32_8 { kind: SignExtend })?
        }
        Operator::I32Load8U { memarg } => {
            load(ctx, memarg, I32, LoadKind::I32_8 { kind: ZeroExtend })?
        }
        Operator::I32Load16S { memarg } => {
            load(ctx, memarg, I32, LoadKind::I32_16 { kind: SignExtend })?
        }
        Operator::I32Load16U { memarg } => {
            load(ctx, memarg, I32, LoadKind::I32_16 { kind: ZeroExtend })?
        }
        Operator::I64Load8S { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_8 { kind: SignExtend })?
        }
        Operator::I64Load8U { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_8 { kind: ZeroExtend })?
        }
        Operator::I64Load16S { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_16 { kind: SignExtend })?
        }
        Operator::I64Load16U { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_16 { kind: ZeroExtend })?
        }
        Operator::I64Load32S { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_32 { kind: SignExtend })?
        }
        Operator::I64Load32U { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64_32 { kind: ZeroExtend })?
        }

        Operator::I32Store { memarg } => store(ctx, memarg, I32, StoreKind::I32 { atomic: false })?,
        Operator::I64Store { memarg } => store(ctx, memarg, I64, StoreKind::I64 { atomic: false })?,
        Operator::F32Store { memarg } => store(ctx, memarg, F32, StoreKind::F32)?,
        Operator::F64Store { memarg } => store(ctx, memarg, F64, StoreKind::F64)?,
        Operator::V128Store { memarg } => store(ctx, memarg, V128, StoreKind::V128)?,
        Operator::I32Store8 { memarg } => {
            store(ctx, memarg, I32, StoreKind::I32_8 { atomic: false })?
        }
        Operator::I32Store16 { memarg } => {
            store(ctx, memarg, I32, StoreKind::I32_16 { atomic: false })?
        }
        Operator::I64Store8 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_8 { atomic: false })?
        }
        Operator::I64Store16 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_16 { atomic: false })?
        }
        Operator::I64Store32 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_32 { atomic: false })?
        }

        Operator::I32AtomicLoad { memarg } => {
            load(ctx, memarg, I32, LoadKind::I32 { atomic: true })?
        }
        Operator::I64AtomicLoad { memarg } => {
            load(ctx, memarg, I64, LoadKind::I64 { atomic: true })?
        }
        Operator::I32AtomicLoad8U { memarg } => load(
            ctx,
            memarg,
            I32,
            LoadKind::I32_8 {
                kind: ZeroExtendAtomic,
            },
        )?,
        Operator::I32AtomicLoad16U { memarg } => load(
            ctx,
            memarg,
            I32,
            LoadKind::I32_16 {
                kind: ZeroExtendAtomic,
            },
        )?,
        Operator::I64AtomicLoad8U { memarg } => load(
            ctx,
            memarg,
            I64,
            LoadKind::I64_8 {
                kind: ZeroExtendAtomic,
            },
        )?,
        Operator::I64AtomicLoad16U { memarg } => load(
            ctx,
            memarg,
            I64,
            LoadKind::I64_16 {
                kind: ZeroExtendAtomic,
            },
        )?,
        Operator::I64AtomicLoad32U { memarg } => load(
            ctx,
            memarg,
            I64,
            LoadKind::I64_32 {
                kind: ZeroExtendAtomic,
            },
        )?,

        Operator::I32AtomicStore { memarg } => {
            store(ctx, memarg, I32, StoreKind::I32 { atomic: true })?
        }
        Operator::I64AtomicStore { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64 { atomic: true })?
        }
        Operator::I32AtomicStore8 { memarg } => {
            store(ctx, memarg, I32, StoreKind::I32_8 { atomic: true })?
        }
        Operator::I32AtomicStore16 { memarg } => {
            store(ctx, memarg, I32, StoreKind::I32_16 { atomic: true })?
        }
        Operator::I64AtomicStore8 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_8 { atomic: true })?
        }
        Operator::I64AtomicStore16 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_16 { atomic: true })?
        }
        Operator::I64AtomicStore32 { memarg } => {
            store(ctx, memarg, I64, StoreKind::I64_32 { atomic: true })?
        }

        Operator::I32AtomicRmwAdd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Add, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwAdd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Add, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UAdd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Add, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UAdd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Add, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UAdd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Add, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UAdd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Add, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UAdd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Add, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwSub { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Sub, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwSub { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Sub, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8USub { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Sub, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16USub { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Sub, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8USub { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Sub, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16USub { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Sub, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32USub { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Sub, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwAnd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::And, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwAnd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::And, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UAnd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::And, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UAnd { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::And, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UAnd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::And, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UAnd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::And, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UAnd { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::And, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwOr { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Or, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwOr { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Or, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UOr { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Or, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UOr { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Or, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UOr { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Or, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UOr { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Or, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UOr { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Or, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwXor { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xor, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwXor { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xor, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UXor { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xor, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UXor { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xor, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UXor { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xor, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UXor { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xor, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UXor { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xor, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwXchg { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xchg, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwXchg { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xchg, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UXchg { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xchg, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UXchg { memarg } => {
            atomicrmw(ctx, memarg, I32, AtomicOp::Xchg, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UXchg { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xchg, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UXchg { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xchg, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UXchg { memarg } => {
            atomicrmw(ctx, memarg, I64, AtomicOp::Xchg, AtomicWidth::I64_32)?;
        }

        Operator::I32AtomicRmwCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I32, AtomicWidth::I32)?;
        }
        Operator::I64AtomicRmwCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I64, AtomicWidth::I64)?;
        }
        Operator::I32AtomicRmw8UCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I32, AtomicWidth::I32_8)?;
        }
        Operator::I32AtomicRmw16UCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I32, AtomicWidth::I32_16)?;
        }
        Operator::I64AtomicRmw8UCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I64, AtomicWidth::I64_8)?;
        }
        Operator::I64AtomicRmw16UCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I64, AtomicWidth::I64_16)?;
        }
        Operator::I64AtomicRmw32UCmpxchg { memarg } => {
            cmpxchg(ctx, memarg, I64, AtomicWidth::I64_32)?;
        }
        Operator::Wake { ref memarg } => {
            let (_, count) = ctx.pop_operand_expected(Some(I32))?;
            let (_, address) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(AtomicNotify {
                count,
                address,
                memory,
                arg: mem_arg(memarg)?,
            });
            ctx.push_operand(Some(I32), expr);
        }
        Operator::I32Wait { ref memarg } | Operator::I64Wait { ref memarg } => {
            let (ty, sixty_four) = match inst {
                Operator::I32Wait { .. } => (I32, false),
                _ => (I64, true),
            };
            let (_, timeout) = ctx.pop_operand_expected(Some(I64))?;
            let (_, expected) = ctx.pop_operand_expected(Some(ty))?;
            let (_, address) = ctx.pop_operand_expected(Some(I32))?;
            let memory = ctx.indices.get_memory(0)?;
            let expr = ctx.func.alloc(AtomicWait {
                timeout,
                expected,
                sixty_four,
                address,
                memory,
                arg: mem_arg(memarg)?,
            });
            ctx.push_operand(Some(I32), expr);
        }

        op => bail!("Have not implemented support for opcode yet: {:?}", op),
    }
    Ok(())
}
