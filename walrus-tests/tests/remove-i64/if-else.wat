(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a") (param $addr i32)
    i32.const 0
    if (result i64)
      global.get 0
    else
      global.get 1
    end
    global.set 2))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32)))
    (func $f0 (type 0) (param i32)
      (local $block_low1 i32) (local $temp_low2 i32) (local $block_high3 i32) (local $block_low4 i32) (local $temp_low5 i32) (local $block_high6 i32) (local $if_else_low7 i32) (local $if_else_high8 i32)
      block  ;; label = @1
        i32.const 0
        if (result i32)  ;; label = @2
          block (result i32)  ;; label = @3
            global.get 0
            local.set $temp_low2
            global.get 1
          end
          local.set $block_high3
          local.get $temp_low2
          local.set $block_low1
          local.get $block_high3
          local.set $if_else_high8
          local.get $block_low1
          local.set $if_else_low7
          local.get $if_else_high8
        else
          block (result i32)  ;; label = @3
            global.get 2
            local.set $temp_low5
            global.get 3
          end
          local.set $block_high6
          local.get $temp_low5
          local.set $block_low4
          local.get $block_high6
          local.set $if_else_high8
          local.get $block_low4
          local.set $if_else_low7
          local.get $if_else_high8
        end
        global.set 5
        local.get $if_else_low7
        global.set 4
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (global (;4;) (mut i32) (i32.const 0))
    (global (;5;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
