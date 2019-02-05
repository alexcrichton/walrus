(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (func (export "a") (param $addr i32)
    local.get 0
    i64.load8_u
    global.set 0))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32)))
    (func $f0 (type 0) (param $addr i32)
      (local $temp_low1 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          local.get $addr
          i32.load8_s
          local.set $temp_low1
          i32.const 0
        end
        global.set 1
        local.get $temp_low1
        global.set 0
      end)
    (memory (;0;) 1)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
