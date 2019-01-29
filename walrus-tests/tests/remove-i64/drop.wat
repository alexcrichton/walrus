(module
  (global (mut i64) (i64.const 0))
  (global (mut i32) (i32.const 0))
  (func (export "a")
    global.get 0
    drop))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low_0 i32)
      block (result i32)  ;; label = @1
        global.get 0
        local.set $temp_low_0
        global.get 1
      end
      drop)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
