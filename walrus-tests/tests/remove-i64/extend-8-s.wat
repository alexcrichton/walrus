(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a")
    global.get 0
    i64.extend8_s
    global.set 1))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $temp_low0 i32) (local $extend1 i32) (local $temp_low2 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          block (result i32)  ;; label = @3
            block (result i32)  ;; label = @4
              global.get 0
              local.set $temp_low0
              global.get 1
            end
            drop
            local.get $temp_low0
          end
          i32.extend8_s
          local.tee $extend1
          local.set $temp_low2
          local.get $extend1
          i32.const 31
          i32.shr_s
        end
        global.set 3
        local.get $temp_low2
        global.set 2
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
