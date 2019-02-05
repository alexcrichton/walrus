(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a") (param $addr i32)
    global.get 0
    i64.ctz
    global.set 1))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32)))
    (func $f0 (type 0) (param i32)
      (local $temp_low1 i32) (local $ctz2 i32) (local $temp_low3 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          block (result i32)  ;; label = @3
            block (result i32)  ;; label = @4
              global.get 0
              local.set $temp_low1
              global.get 1
            end
            local.set $ctz2
            local.get $temp_low1
            i32.ctz
            i32.const 32
            local.get $ctz2
            i32.ctz
            i32.add
            local.get $temp_low1
            i32.eqz
            select
          end
          local.set $temp_low3
          i32.const 0
        end
        global.set 3
        local.get $temp_low3
        global.set 2
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
