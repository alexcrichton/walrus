(module
  (global (mut i64) (i64.const 1))
  (func (export "a")
    global.get 0
    global.set 0))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low_0 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          global.get 0
          local.set $temp_low_0
          global.get 1
        end
        global.set 1
        local.get $temp_low_0
        global.set 0
      end)
    (global (;0;) (mut i32) (i32.const 1))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
