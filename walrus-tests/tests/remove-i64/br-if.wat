(module
  (memory 1)
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (global (mut i64) (i64.const 0))
  (func (export "a") (param $addr i32)
    block (result i64)
      global.get 0
      i32.const 0
      br_if 0
      global.set 1
      global.get 2
    end
    global.set 3))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32)))
    (func $f0 (type 0) (param i32)
      (local $block_low2 i32) (local $block_low3 i32) (local $temp_low4 i32) (local $br_if_val1_low i32) (local $br_if_val1_high i32) (local $block_low7 i32) (local $temp_low8 i32) (local $br_high9 i32) (local $block_low11 i32) (local $temp_low12 i32) (local $block_high13 i32) (local $if_else_low14 i32) (local $if_else_high15 i32) (local $block_high16 i32) (local $temp_low17 i32) (local $block_high18 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          block  ;; label = @3
            block (result i32)  ;; label = @4
              block  ;; label = @5
                block (result i32)  ;; label = @6
                  global.get 0
                  local.set $temp_low4
                  global.get 1
                end
                local.set $br_if_val1_high
                local.get $temp_low4
                local.set $br_if_val1_low
              end
              i32.const 0
              if (result i32)  ;; label = @5
                block (result i32)  ;; label = @6
                  block (result i32)  ;; label = @7
                    block (result i32)  ;; label = @8
                      local.get $br_if_val1_low
                      local.set $temp_low8
                      local.get $br_if_val1_high
                    end
                    local.set $br_high9
                    local.get $temp_low8
                    local.set $block_low2
                    local.get $br_high9
                    br 4 (;@3;)
                  end
                  local.set $if_else_high15
                  local.get $block_low7
                  local.set $if_else_low14
                  local.get $if_else_high15
                end
              else
                block (result i32)  ;; label = @6
                  block (result i32)  ;; label = @7
                    local.get $br_if_val1_low
                    local.set $temp_low12
                    local.get $br_if_val1_high
                  end
                  local.set $block_high13
                  local.get $temp_low12
                  local.set $block_low11
                  local.get $block_high13
                  local.set $if_else_high15
                  local.get $block_low11
                  local.set $if_else_low14
                  local.get $if_else_high15
                end
              end
              local.set $block_high16
              local.get $if_else_low14
              local.set $block_low3
              local.get $block_high16
            end
            global.set 3
            local.get $block_low3
            global.set 2
          end
          block (result i32)  ;; label = @3
            global.get 4
            local.set $temp_low17
            global.get 5
          end
          local.set $block_high18
          local.get $temp_low17
          local.set $block_low2
          local.get $block_high18
        end
        global.set 7
        local.get $block_low2
        global.set 6
      end)
    (global (;0;) (mut i32) (i32.const 0))
    (global (;1;) (mut i32) (i32.const 0))
    (global (;2;) (mut i32) (i32.const 0))
    (global (;3;) (mut i32) (i32.const 0))
    (global (;4;) (mut i32) (i32.const 0))
    (global (;5;) (mut i32) (i32.const 0))
    (global (;6;) (mut i32) (i32.const 0))
    (global (;7;) (mut i32) (i32.const 0))
    (export "a" (func $f0)))
;)
