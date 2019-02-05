(module
  (global (mut i64) (i64.const 0))
  (func (export "a") (param f64)
    local.get 0
    i64.reinterpret_f64
    global.set 0))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param f64)))
    (func $f0 (type 0) (param $arg0 f64)
      (local $block_low1 i32) (local $load_address2 i32) (local $temp_low3 i32) (local $block_high4 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          i32.const 0
          local.get $arg0
          f64.store
          block (result i32)  ;; label = @3
            i32.const 0
            local.tee $load_address2
            i32.load
            local.set $temp_low3
            local.get $load_address2
            i32.load offset=4
          end
          local.set $block_high4
          local.get $temp_low3
          local.set $block_low1
          local.get $block_high4
        end
        global.set 1
        local.get $block_low1
        global.set 0
      end)
    (memory (;0;) 1 1)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
