(module
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a")
    (local i64)
    global.get 0
    global.get 1
    i64.xor
    global.set 0))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low1 i32) (local $temp_low2 i32) (local $binop_lhs_high3 i32) (local $binop_rhs_high4 i32) (local $temp_low5 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          block (result i32)  ;; label = @3
            block (result i32)  ;; label = @4
              global.get 0
              local.set $temp_low1
              global.get 1
            end
            local.set $binop_lhs_high3
            block (result i32)  ;; label = @4
              global.get 2
              local.set $temp_low2
              global.get 3
            end
            local.set $binop_rhs_high4
            local.get $temp_low1
            local.get $temp_low2
            i32.xor
          end
          local.set $temp_low5
          local.get $binop_lhs_high3
          local.get $binop_rhs_high4
          i32.xor
        end
        global.set 1
        local.get $temp_low5
        global.set 0
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
