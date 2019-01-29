#![allow(missing_docs)]

use crate::error::Result;
use crate::ir::*;
use crate::map::IdHashMap;
use crate::module::functions::FunctionId;
use crate::module::functions::LocalFunction;
use crate::module::globals::{Global, GlobalKind};
use crate::module::locals::ModuleLocals;
use crate::module::memories::MemoryId;
use crate::module::{Module, ModuleConfig};
use crate::ty::ValType;
use failure::bail;
use id_arena::Id;
use std::cmp;
use std::mem;

pub fn run(module: &mut Module) -> Result<()> {
    let mut analysis = Analysis::default();
    analysis.split_globals(module)?;

    // lowering might require a memory, so if one isn't already here then we go
    // ahead and add one. If one is already here then we assume address 0 and
    // near are not used.
    let memory = module.memories.iter().next().map(|m| m.id());
    let memory = memory.unwrap_or_else(|| module.memories.add_local(false, 1, Some(1)));
    let locals = &mut module.locals;
    let config = &module.config;
    module.funcs.iter_local_mut().for_each(|(id, func)| {
        let mut entry = func.entry_block();

        // First, remove a various number of 64-bit operations by lowering them
        // to "simpler" alternatives. The next pass will panic if these
        // operations still exist in the IR.
        LowerI64 {
            memory,
            func,
            replace_with: None,
            id: entry.into(),
            locals,
        }
        .visit_block_id_mut(&mut entry);

        // And now that we've pruned our IR a bit, fully delete the i64 types.
        RemoveI64 {
            func_id: id,
            id: entry.into(),
            func,
            analysis: &analysis,
            replace_with: None,
            locals,
            low_bits: IdHashMap::default(),
            local_halves: IdHashMap::default(),
            memory,
            config,
        }
        .visit_block_id_mut(&mut entry);
    });

    Ok(())
}

#[derive(Default)]
struct Analysis {
    globals: IdHashMap<Global, Replace<Global>>,
}

struct Replace<T> {
    low: Id<T>,
    high: Id<T>,
}

impl<T> Clone for Replace<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Replace<T> {}

impl Analysis {
    fn split_globals(&mut self, module: &mut Module) -> Result<()> {
        use crate::const_value::Const;

        let mut to_split = Vec::new();
        for global in module.globals.iter() {
            match global.ty {
                ValType::I64 => {}
                _ => continue,
            }
            let val = match global.kind {
                GlobalKind::Import(_) | GlobalKind::Local(Const::Global(_)) => {
                    bail!("can't import 64-bit globals")
                }
                GlobalKind::Local(Const::Value(val)) => val,
            };
            let val = match val {
                Value::I64(n) => n,
                _ => bail!("type mismatch in globals"),
            };
            to_split.push((global.id(), val, global.mutable));
        }

        for (id, val, mutable) in to_split {
            let low = Const::Value(Value::I32(val as i32));
            let high = Const::Value(Value::I32((val >> 32) as i32));
            let low = module.globals.add_local(ValType::I32, mutable, low);
            let high = module.globals.add_local(ValType::I32, mutable, high);
            self.globals.insert(id, Replace { low, high });
        }

        Ok(())
    }
}

struct LowerI64<'a> {
    memory: MemoryId,
    func: &'a mut LocalFunction,
    replace_with: Option<ExprId>,
    id: ExprId,
    locals: &'a mut ModuleLocals,
}

impl LowerI64<'_> {
    /// Flags that the current expression being visited should be replaced with
    /// `id`.
    ///
    /// This should only be called *after* the child nodes have been visited.
    fn replace_with(&mut self, id: ExprId) {
        assert!(self.replace_with.is_none());
        self.replace_with = Some(id);
    }
}

impl VisitorMut for LowerI64<'_> {
    fn local_function_mut(&mut self) -> &mut LocalFunction {
        self.func
    }

    fn visit_expr_id_mut(&mut self, expr: &mut ExprId) {
        assert!(self.replace_with.is_none());
        let prev = mem::replace(&mut self.id, *expr);
        expr.visit_mut(self);
        if let Some(id) = self.replace_with.take() {
            *expr = id;
        }
        self.id = prev;
    }

    fn visit_unop_mut(&mut self, expr: &mut Unop) {
        self.visit_expr_id_mut(&mut expr.expr);

        match expr.op {
            // Replace *64.reinterpret_*64 with a memory load/store through
            // address zero. Right now it's not clear if there's a better way to
            // do this, but it should work for now! In any case this means that
            // `RemoveI64` doesn't have to handle these ops.
            UnaryOp::F64ReinterpretI64 => {
                let zero = self.func.const_(Value::I32(0));
                let arg = MemArg::new(8);
                let store = self.func.store(
                    self.memory,
                    StoreKind::I64 { atomic: false },
                    arg,
                    zero,
                    expr.expr,
                );
                let zero = self.func.const_(Value::I32(0));
                let load = self.func.load(self.memory, LoadKind::F64, arg, zero);
                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::F64]),
                    exprs: vec![store, load],
                });
                self.replace_with(block.into());
            }
            UnaryOp::I64ReinterpretF64 => {
                let zero = self.func.const_(Value::I32(0));
                let arg = MemArg::new(8);
                let store = self
                    .func
                    .store(self.memory, StoreKind::F64, arg, zero, expr.expr);
                let zero = self.func.const_(Value::I32(0));
                let load = self
                    .func
                    .load(self.memory, LoadKind::I64 { atomic: false }, arg, zero);
                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::I64]),
                    exprs: vec![store, load],
                });
                self.replace_with(block.into());
            }

            // Replace extensions of 8/16 -> 64 with an extension of 32 -> 64
            // so the lowering below only has to handle one case.
            UnaryOp::I64Extend8S | UnaryOp::I64Extend16S => {
                expr.op = match expr.op {
                    UnaryOp::I64Extend8S => UnaryOp::I32Extend8S,
                    _ => UnaryOp::I32Extend16S,
                };
                expr.expr = self.func.unop(UnaryOp::I32WrapI64, expr.expr);
                let extend = self.func.unop(UnaryOp::I64ExtendSI32, self.id);
                self.replace_with(extend);
            }

            _ => {}
        }
    }

    /// Canonicalize all loads of 64-bit values to either a full load or a load
    /// of a 32-bit value followed by an extend.
    fn visit_load_mut(&mut self, load: &mut Load) {
        load.visit_mut(self);

        let (new_load_kind, extend) = match load.kind {
            LoadKind::I64_8 {
                kind: ExtendedLoad::SignExtend,
            } => (
                LoadKind::I32_8 {
                    kind: ExtendedLoad::SignExtend,
                },
                UnaryOp::I64ExtendSI32,
            ),
            LoadKind::I64_8 {
                kind: ExtendedLoad::ZeroExtend,
            } => (
                LoadKind::I32_8 {
                    kind: ExtendedLoad::SignExtend,
                },
                UnaryOp::I64ExtendUI32,
            ),
            LoadKind::I64_16 {
                kind: ExtendedLoad::SignExtend,
            } => (
                LoadKind::I32_16 {
                    kind: ExtendedLoad::SignExtend,
                },
                UnaryOp::I64ExtendSI32,
            ),
            LoadKind::I64_16 {
                kind: ExtendedLoad::ZeroExtend,
            } => (
                LoadKind::I32_16 {
                    kind: ExtendedLoad::SignExtend,
                },
                UnaryOp::I64ExtendUI32,
            ),
            LoadKind::I64_32 {
                kind: ExtendedLoad::SignExtend,
            } => (LoadKind::I32 { atomic: false }, UnaryOp::I64ExtendSI32),
            LoadKind::I64_32 {
                kind: ExtendedLoad::ZeroExtend,
            } => (LoadKind::I32 { atomic: false }, UnaryOp::I64ExtendUI32),
            _ => return,
        };

        load.kind = new_load_kind;
        let extend = self.func.unop(extend, self.id);;
        self.replace_with(extend);
    }

    /// Canonicalize all stores of 64-bit values to either a full 64-bits or a
    /// 32-bit store with a wrapped value.
    fn visit_store_mut(&mut self, store: &mut Store) {
        store.visit_mut(self);

        let new_store_kind = match store.kind {
            StoreKind::I64_8 { atomic: false } => StoreKind::I32_8 { atomic: false },
            StoreKind::I64_16 { atomic: false } => StoreKind::I32_16 { atomic: false },
            StoreKind::I64_32 { atomic: false } => StoreKind::I32 { atomic: false },
            _ => return,
        };

        let low_32 = self.func.unop(UnaryOp::I32WrapI64, store.value);
        store.value = low_32;
        store.kind = new_store_kind;
    }
}

struct RemoveI64<'a> {
    func: &'a mut LocalFunction,
    func_id: FunctionId,
    id: ExprId,
    analysis: &'a Analysis,
    replace_with: Option<ExprId>,
    low_bits: IdHashMap<Expr, LocalId>,
    locals: &'a mut ModuleLocals,
    local_halves: IdHashMap<Local, Replace<Local>>,
    memory: MemoryId,
    config: &'a ModuleConfig,
}

impl RemoveI64<'_> {
    /// Returns the two 32-bit locals used for the 64-bit local specified.
    ///
    /// If the `local` hasn't already been split then this function goes ahead
    /// and splits it, assigning new low/high bit locals for it.
    ///
    /// The `local` specified must be of type `i64` and the two returned locals
    /// are of type `i32`
    fn local_halves(&mut self, local: LocalId) -> Replace<Local> {
        if let Some(pair) = self.local_halves.get(&local) {
            return *pair;
        }
        let replace = Replace {
            low: self.locals.add(ValType::I32),
            high: self.locals.add(ValType::I32),
        };
        if self.config.generate_names {
            let mut base = self.locals.get(local).name.clone().unwrap_or(String::new());
            if base.is_empty() {
                base.push_str(&local.index().to_string());
            }
            self.locals.get_mut(replace.low).name = Some(format!("{}_low", base));
            self.locals.get_mut(replace.high).name = Some(format!("{}_high", base));
        }
        self.local_halves.insert(local, replace);
        replace
    }

    /// Spill the expression `bits` into a 32-bit local, returning the
    /// `local.set` instruction as well as the local we spilled to.
    fn spill(&mut self, bits: ExprId) -> (ExprId, LocalId) {
        let local = self.locals.add(ValType::I32);
        if self.config.generate_names {
            let idx = self.low_bits.len();
            self.locals.get_mut(local).name = Some(format!("temp_low_{}", idx));
        }
        let local_set = self.func.local_set(local, bits);
        (local_set, local)
    }

    /// Replace the current instruction with the two halves specified.
    ///
    /// This is intended to be used on instructions that produce 64-bit values.
    /// The `low_bits` value here is the expression representing the low bits
    /// of the computation, and `high_bits` is the high bits.
    ///
    /// This will replace the current expression with a block that evaluates
    /// the set of the `low_bits` followed by the evaluation of the high bits.
    /// Note that `low_bits` is evaluated before `high_bits`!
    fn split(&mut self, low_bits: ExprId, high_bits: ExprId) {
        let (local_set, local) = self.spill(low_bits);
        let block = self.func.alloc(Block {
            kind: BlockKind::Block,
            params: Box::new([]),
            results: Box::new([ValType::I32]),
            exprs: vec![local_set, high_bits],
        });
        self.replace_with(block.into());
        self.low_bits.insert(block.into(), local);
    }

    /// Consumes the two expressions specified and replaces the current
    /// expression with a block of `a` and `b`.
    ///
    /// Only for use with expressions which don't have a value
    fn consume(&mut self, a: ExprId, b: ExprId) {
        let block = self.func.alloc(Block {
            kind: BlockKind::Block,
            params: Box::new([]),
            results: Box::new([]),
            exprs: vec![a, b],
        });
        self.replace_with(block.into());
    }

    /// Flags that the current expression being visited should be replaced with
    /// `id`.
    ///
    /// This should only be called *after* the child nodes have been visited.
    fn replace_with(&mut self, id: ExprId) {
        assert!(self.replace_with.is_none());
        self.replace_with = Some(id);
    }

    /// Replaces a 64-bit bitwise operation with two 32-bit components.
    fn binary_bitop(&mut self, expr: &mut Binop, op32: BinaryOp) {
        let lhs_temp_high = self.locals.add(ValType::I32);
        let rhs_temp_high = self.locals.add(ValType::I32);

        if self.config.generate_names {
            self.locals.get_mut(lhs_temp_high).name = Some(format!("lhs_temp_high"));
            self.locals.get_mut(rhs_temp_high).name = Some(format!("rhs_temp_high"));
        }

        let lhs_temp = self.func.local_set(lhs_temp_high, expr.lhs);
        let rhs_temp = self.func.local_set(rhs_temp_high, expr.rhs);

        let lhs_low = self.low_bits[&expr.lhs];
        let rhs_low = self.low_bits[&expr.rhs];

        let lhs = self.func.local_get(lhs_low);
        let rhs = self.func.local_get(rhs_low);

        let low = self.func.binop(op32, lhs, rhs);
        let low = self.func.alloc(Block {
            kind: BlockKind::Block,
            params: Box::new([]),
            results: Box::new([ValType::I32]),
            exprs: vec![lhs_temp, rhs_temp, low],
        });
        expr.op = op32;
        expr.lhs = self.func.local_get(lhs_temp_high);
        expr.rhs = self.func.local_get(rhs_temp_high);
        self.split(low.into(), self.id);
    }
}

impl VisitorMut for RemoveI64<'_> {
    fn local_function_mut(&mut self) -> &mut LocalFunction {
        self.func
    }

    fn visit_expr_id_mut(&mut self, expr: &mut ExprId) {
        assert!(self.replace_with.is_none());
        let prev = mem::replace(&mut self.id, *expr);
        expr.visit_mut(self);
        if let Some(id) = self.replace_with.take() {
            *expr = id;
        }
        self.id = prev;
    }

    fn visit_global_get_mut(&mut self, expr: &mut GlobalGet) {
        expr.visit_mut(self);

        let replace = match self.analysis.globals.get(&expr.global) {
            Some(r) => r,
            None => return,
        };
        // Turn this `expr` into a fetch of the low bits, allocate a new expr
        // for a fetch of the high bits, and then split with those two exprs
        expr.global = replace.low;
        let high_bits = self.func.global_get(replace.high);
        self.split(self.id, high_bits);
    }

    fn visit_global_set_mut(&mut self, expr: &mut GlobalSet) {
        expr.visit_mut(self);

        let replace = match self.analysis.globals.get(&expr.global) {
            Some(r) => r,
            None => return,
        };

        // The `expr.value` expression is the high bits of the value along with
        // the computation tree, so execute that first by updating where this
        // expression stores into.
        expr.global = replace.high;

        // Afterwards we need to fetch the local with the low bits and then
        // store that into the high bits of the global.
        let local = self.low_bits[&expr.value];
        let low_bits = self.func.local_get(local);
        let low_bits = self.func.global_set(replace.low, low_bits);
        self.consume(self.id, low_bits);
    }

    fn visit_local_get_mut(&mut self, expr: &mut LocalGet) {
        expr.visit_mut(self);

        if self.locals.get(expr.local).ty() != ValType::I64 {
            return;
        }
        // See `global.get` for more info, this is the same as that basically
        let replace = self.local_halves(expr.local);
        expr.local = replace.low;
        let high_bits = self.func.local_get(replace.high);
        self.split(self.id, high_bits);
    }

    fn visit_local_set_mut(&mut self, expr: &mut LocalSet) {
        expr.visit_mut(self);

        if self.locals.get(expr.local).ty() != ValType::I64 {
            return;
        }
        // See `global.get` for more info, this is the same as that basically
        let replace = self.local_halves(expr.local);
        expr.local = replace.high;
        let local = self.low_bits[&expr.value];
        let low_bits = self.func.local_get(local);
        let low_bits = self.func.local_set(replace.low, low_bits);
        self.consume(self.id, low_bits);
    }

    fn visit_local_tee_mut(&mut self, expr: &mut LocalTee) {
        expr.visit_mut(self);
        if self.locals.get(expr.local).ty() != ValType::I64 {
            return;
        }

        // Transform into:
        //
        //  (block (result i32)
        //      (block
        //          (local.set $local_high ($high_bits))
        //          (local.set $tmp
        //              (local.tee $local_low (local.get $low_bits))))
        //      (local.get $local_high))
        //
        // The basic diea is that we evaluate the high bits into the actual
        // local's own high bits, then we tee the low bits from our expression's
        // temporary as well as a new temporary. Then we fetch the high bits
        // local again to polish it all off.

        let replace = self.local_halves(expr.local);
        let set_high = self.func.local_set(replace.high, expr.value);
        let low_temp = self.low_bits[&expr.value];
        let get_low_temp = self.func.local_get(low_temp);
        let tee_low = self.func.local_tee(replace.low, get_low_temp);
        let get_high = self.func.local_get(replace.high);

        let block = self.func.alloc(Block {
            kind: BlockKind::Block,
            params: Box::new([]),
            results: Box::new([ValType::I32]),
            exprs: vec![set_high, tee_low],
        });
        self.split(block.into(), get_high);
    }

    fn visit_const_mut(&mut self, const_: &mut Const) {
        const_.visit_mut(self);
        let val = match const_.value {
            Value::I64(val) => val,
            _ => return,
        };

        let low = self.func.const_(Value::I32(val as i32));
        let high = self.func.const_(Value::I32((val >> 32) as i32));
        self.split(low, high);
    }

    fn visit_call_mut(&mut self, call: &mut Call) {
        call.visit_mut(self);
        unimplemented!()
    }

    fn visit_call_indirect_mut(&mut self, call: &mut CallIndirect) {
        call.visit_mut(self);
        unimplemented!()
    }

    fn visit_select_mut(&mut self, select: &mut Select) {
        select.visit_mut(self);
        unimplemented!()
    }

    fn visit_br_mut(&mut self, br: &mut Br) {
        br.visit_mut(self);
        unimplemented!()
    }

    fn visit_br_if_mut(&mut self, br_if: &mut BrIf) {
        br_if.visit_mut(self);
        unimplemented!()
    }

    fn visit_br_table_mut(&mut self, br_table: &mut BrTable) {
        br_table.visit_mut(self);
        unimplemented!()
    }

    fn visit_if_else_mut(&mut self, expr: &mut IfElse) {
        expr.visit_mut(self);
        unimplemented!()
    }

    fn visit_return_mut(&mut self, expr: &mut Return) {
        expr.visit_mut(self);
        unimplemented!()
    }

    fn visit_binop_mut(&mut self, expr: &mut Binop) {
        expr.visit_mut(self);

        match expr.op {
            BinaryOp::I64Eq
            | BinaryOp::I64Ne
            | BinaryOp::I64LtS
            | BinaryOp::I64LtU
            | BinaryOp::I64GtS
            | BinaryOp::I64GtU
            | BinaryOp::I64LeS
            | BinaryOp::I64LeU
            | BinaryOp::I64GeS
            | BinaryOp::I64GeU
            | BinaryOp::I64Add
            | BinaryOp::I64Sub
            | BinaryOp::I64Mul
            | BinaryOp::I64DivS
            | BinaryOp::I64DivU
            | BinaryOp::I64RemS
            | BinaryOp::I64RemU
            | BinaryOp::I64Shl
            | BinaryOp::I64ShrS
            | BinaryOp::I64ShrU
            | BinaryOp::I64Rotl
            | BinaryOp::I64Rotr => unimplemented!(),

            BinaryOp::I64And => self.binary_bitop(expr, BinaryOp::I32And),
            BinaryOp::I64Or => self.binary_bitop(expr, BinaryOp::I32Or),
            BinaryOp::I64Xor => self.binary_bitop(expr, BinaryOp::I32Xor),

            _ => return,
        }
    }

    fn visit_unop_mut(&mut self, expr: &mut Unop) {
        expr.visit_mut(self);

        match expr.op {
            UnaryOp::F32ConvertSI64
            | UnaryOp::F32ConvertUI64
            | UnaryOp::F64ConvertSI64
            | UnaryOp::F64ConvertUI64
            | UnaryOp::I64TruncSF32
            | UnaryOp::I64TruncUF32
            | UnaryOp::I64TruncSF64
            | UnaryOp::I64TruncUF64 => unimplemented!(),

            // Should have been handled in the above `LowerI64`
            UnaryOp::F64ReinterpretI64
            | UnaryOp::I64ReinterpretF64
            | UnaryOp::I64Extend8S
            | UnaryOp::I64Extend16S => unreachable!(),

            UnaryOp::I64ExtendUI32 => {
                // Pretty easy, the high bits are always zero!
                let zero = self.func.const_(Value::I32(0));
                self.split(expr.expr, zero)
            }

            UnaryOp::I64ExtendSI32 => {
                // We'll want to take the expression and unconditionally move
                // them to the low bits. The upper 32-bits are the 31st bit of
                // the low bits, broadcast to all bits (a signed shift right).
                let local = self.locals.add(ValType::I32);
                let tee_low = self.func.local_tee(local, expr.expr);
                let amt = self.func.const_(Value::I32(31));
                let get_low = self.func.local_get(local);
                let shift = self.func.binop(BinaryOp::I32ShrS, get_low, amt);
                self.split(tee_low, shift)
            }

            UnaryOp::I64Extend32S => {
                // Same as above, but our low bits are slightly different
                let local = self.locals.add(ValType::I32);
                let low = self.low_bits[&expr.expr];
                let load_low = self.func.local_get(low);
                let drop_high = self.func.drop(expr.expr);
                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::I32]),
                    exprs: vec![drop_high, load_low],
                });
                let tee_low = self.func.local_tee(local, block.into());
                let amt = self.func.const_(Value::I32(31));
                let get_low = self.func.local_get(local);
                let shift = self.func.binop(BinaryOp::I32ShrS, get_low, amt);
                self.split(tee_low, shift)
            }

            UnaryOp::I64Eqz => {
                // Turn ourselves into a 32-bit eqz, eqz the low bits, then or
                // the result.
                let low = self.low_bits[&expr.expr];
                expr.op = UnaryOp::I32Eqz;

                let low = self.func.local_get(low);
                let rhs = self.func.unop(UnaryOp::I32Eqz, low);
                let result = self.func.binop(BinaryOp::I32And, self.id, rhs);
                self.replace_with(result);
            }

            UnaryOp::I64Popcnt => {
                // Turn ourselves into a 32-bit popcnt, and then our low bits
                // are added to our own popcnt (the high bits) to the popcnt of
                // the low bits (the local stored during computing the high
                // bits)
                let low = self.low_bits[&expr.expr];
                expr.op = UnaryOp::I32Popcnt;

                let low = self.func.local_get(low);
                let rhs = self.func.unop(UnaryOp::I32Popcnt, low);
                let low = self.func.binop(BinaryOp::I32Add, self.id, rhs);

                // Low bits are the add, high bits are always zero as you can't
                // have more than 2^32 bits.
                let zero = self.func.const_(Value::I32(0));
                self.split(low, zero);
            }

            UnaryOp::I32WrapI64 => {
                // Execute the high bits, drop them, and then return the low
                // bits.
                let low = self.low_bits[&expr.expr];
                let drop_high = self.func.drop(expr.expr);
                let load_low = self.func.local_get(low);
                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::I32]),
                    exprs: vec![drop_high, load_low],
                });
                self.replace_with(block.into());
            }

            UnaryOp::I64Ctz => {
                // Mapping roughly to:
                //
                //  (block (result i32)
                //      (local.set $tmp $high_bits)
                //      (select
                //          (i32.eqz (local.get $low))
                //          (i32.add (i32.const 32) (i32.ctz (local.get $high)))
                //          (i32.ctz (local.get $low)))
                //
                // Note that the high bits are always zero as you can't have
                // more than 2^32 bits.

                let high = self.locals.add(ValType::I32);
                let low = self.low_bits[&expr.expr];

                let set_high = self.func.local_set(high, expr.expr);

                let load_low = self.func.local_get(low);
                let condition = self.func.unop(UnaryOp::I32Eqz, load_low);

                let load_high = self.func.local_get(high);
                let ctz_high = self.func.unop(UnaryOp::I32Ctz, load_high);
                let c32 = self.func.const_(Value::I32(32));
                let if_true = self.func.binop(BinaryOp::I32Add, c32, ctz_high);

                let if_false = self.func.unop(UnaryOp::I32Ctz, load_low);
                let select = self.func.select(condition, if_true, if_false);

                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::I32]),
                    exprs: vec![set_high, select],
                });
                let zero = self.func.const_(Value::I32(0));
                self.split(block.into(), zero);
            }

            UnaryOp::I64Clz => {
                // Mapping roughly to:
                //
                //  (block (result i32)
                //      (local.set $tmp $high_bits)
                //      (select
                //          (i32.eqz (local.get $high))
                //          (i32.add (i32.const 32) (i32.clz (local.get $low)))
                //          (i32.ctz (local.get $high)))
                //
                // Note that the high bits are always zero as you can't have
                // more than 2^32 bits.

                let high = self.locals.add(ValType::I32);
                let low = self.low_bits[&expr.expr];

                let set_high = self.func.local_set(high, expr.expr);

                let load_low = self.func.local_get(low);
                let load_high = self.func.local_get(high);

                let condition = self.func.unop(UnaryOp::I32Eqz, load_high);

                let clz_low = self.func.unop(UnaryOp::I32Clz, load_low);
                let c32 = self.func.const_(Value::I32(32));
                let if_true = self.func.binop(BinaryOp::I32Add, c32, clz_low);

                let load_low = self.func.local_get(low);
                let if_false = self.func.unop(UnaryOp::I32Clz, load_high);
                let select = self.func.select(condition, if_true, if_false);

                let block = self.func.alloc(Block {
                    kind: BlockKind::Block,
                    params: Box::new([]),
                    results: Box::new([ValType::I32]),
                    exprs: vec![set_high, select],
                });
                let zero = self.func.const_(Value::I32(0));
                self.split(block.into(), zero);
            }

            _ => return,
        }
    }

    fn visit_load_mut(&mut self, expr: &mut Load) {
        expr.visit_mut(self);

        match expr.kind {
            LoadKind::I64 { atomic: false } => {
                // We'll want to change this into:
                //
                //  (block (result i32)
                //      (local.set $tmp_low
                //          (i32.load (local.tee $tmp ($address))))
                //      (i32.load offset=4 (local.get $tmp)))

                let address_local = self.locals.add(ValType::I32);
                let address = self.func.local_tee(address_local, expr.address);

                let kind = LoadKind::I32 { atomic: false };
                let arg = expr.arg.with_align(cmp::min(expr.arg.align, 4));
                let low = self.func.load(expr.memory, kind, arg, address);

                expr.kind = kind;
                expr.arg = arg.with_offset(expr.arg.offset + 4);
                expr.address = self.func.local_get(address_local);

                self.split(low, self.id)
            }

            // These should be handled by `LowerI64` above
            LoadKind::I64 { .. }
            | LoadKind::I64_8 { .. }
            | LoadKind::I64_16 { .. }
            | LoadKind::I64_32 { .. } => {
                panic!("unimplemented 64-bit atomic loads");
            }

            _ => return,
        }
    }

    fn visit_store_mut(&mut self, expr: &mut Store) {
        expr.visit_mut(self);

        match expr.kind {
            StoreKind::I64 { atomic: false } => {
                // We'll want to change this into:
                //
                //  (block
                //      (i32.store offset=4 (local.tee $tmp ($address)) $high)
                //      (i32.store (local.get $address) (local.get $low)))

                let address_local = self.locals.add(ValType::I32);
                let kind = StoreKind::I32 { atomic: false };
                let arg = expr.arg.with_align(cmp::min(expr.arg.align, 4));

                expr.kind = kind;
                expr.arg = arg.with_offset(expr.arg.offset + 4);
                expr.address = self.func.local_tee(address_local, expr.address);

                let local = self.low_bits[&expr.value];
                let low_bits = self.func.local_get(local);
                let address = self.func.local_get(address_local);
                let low = self.func.store(expr.memory, kind, arg, address, low_bits);

                self.consume(self.id, low);
                return;
            }

            // The nonatomic versions should be handled above.
            StoreKind::I64 { .. }
            | StoreKind::I64_8 { .. }
            | StoreKind::I64_16 { .. }
            | StoreKind::I64_32 { .. } => {
                panic!("unimplemented 64-bit atomic stores");
            }

            _ => return,
        }
    }

    fn visit_block_mut(&mut self, block: &mut Block) {
        block.visit_mut(self);

        if block.results.len() > 1 {
            panic!("unimplemented support for multi-result blocks");
        }

        // If a block has a result type of i64 then our transformation means
        // it'll actually have a result type of i32, so make it so.
        // Additionally we'll store that the local for our expression that
        // contains the low bits is the same as the local for the last
        // expression that contains the low bits.
        if let Some(ValType::I64) = block.results.get(0) {
            block.results[0] = ValType::I32;
            let last = block.exprs.last().unwrap();
            let local = self.low_bits[&last];
            self.low_bits.insert(self.id, local);
        }
    }
}
