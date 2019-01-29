(module
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a")
    global.get 0
    i64.popcnt
    global.set 1))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low_0 i32) (local $temp_low_1 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          block (result i32)  ;; label = @3
            global.get 0
            local.set $temp_low_0
            global.get 1
          end
          i32.popcnt
          local.get $temp_low_0
          i32.popcnt
          i32.add
          local.set $temp_low_1
          i32.const 0
        end
        global.set 3
        local.get $temp_low_1
        global.set 2
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
