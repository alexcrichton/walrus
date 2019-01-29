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
      (local i32 $temp_low_0 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          i32.const 0
          local.get $arg0
          f64.store
          block (result i32)  ;; label = @3
            i32.const 0
            local.tee 1
            i32.load
            local.set $temp_low_0
            local.get 1
            i32.load offset=4
          end
        end
        global.set 1
        local.get $temp_low_0
        global.set 0
      end)
    (memory (;0;) 1 1)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
