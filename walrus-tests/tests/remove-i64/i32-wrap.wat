(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (func (export "a") (result i32)
    global.get 0
    i32.wrap_i64))

(; CHECK-ALL:
  (module
    (type (;0;) (func (result i32)))
    (func $f0 (type 0) (result i32)
      (local $temp_low_0 i32)
      block (result i32)  ;; label = @1
        block (result i32)  ;; label = @2
          global.get 0
          local.set $temp_low_0
          global.get 1
        end
        drop
        local.get $temp_low_0
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
