(module
  (global (mut i64) (i64.const 0))
  (func (export "a") (result f64)
    global.get 0
    f64.reinterpret_i64))

(; CHECK-ALL:
  (module
    (type (;0;) (func (result f64)))
    (func $f0 (type 0) (result f64)
      (local $temp_low_0 i32) (local i32)
      block (result f64)  ;; label = @1
        block  ;; label = @2
          i32.const 0
          local.tee 1
          block (result i32)  ;; label = @3
            global.get 0
            local.set $temp_low_0
            global.get 1
          end
          i32.store offset=4
          local.get 1
          local.get $temp_low_0
          i32.store
        end
        i32.const 0
        f64.load
      end)
    (memory (;0;) 1 1)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
