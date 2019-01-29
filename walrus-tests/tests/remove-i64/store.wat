(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (func (export "a") (param $addr i32)
    local.get 0
    global.get 0
    i64.store))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32)))
    (func $f0 (type 0) (param $addr i32)
      (local $temp_low_0 i32) (local i32)
      block  ;; label = @1
        local.get $addr
        local.tee 2
        block (result i32)  ;; label = @2
          global.get 0
          local.set $temp_low_0
          global.get 1
        end
        i32.store offset=4
        local.get 2
        local.get $temp_low_0
        i32.store
      end)
    (memory (;0;) 1)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
