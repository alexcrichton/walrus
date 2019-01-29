(module
  (global (mut i64) (i64.const 0))
  (global (mut i32) (i32.const 0))
  (func (export "a")
    global.get 0
    i64.eqz
    global.set 1))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low_0 i32)
      block (result i32)  ;; label = @1
        global.get 1
        local.set $temp_low_0
        global.get 2
      end
      i32.eqz
      local.get $temp_low_0
      i32.eqz
      i32.and
      global.set 0)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
