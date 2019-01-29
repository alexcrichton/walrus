(module
  (func (export "a")
    (local i64)
    local.get 0
    local.set 0))

(; CHECK-ALL:
  (module
    (type (;0;) (func))
    (func $f0 (type 0)
      (local $l0_low i32) (local $l0_high i32) (local $temp_low_0 i32)
      block  ;; label = @1
        block (result i32)  ;; label = @2
          local.get $l0_low
          local.set $temp_low_0
          local.get $l0_high
        end
        local.set $l0_high
        local.get $temp_low_0
        local.set $l0_low
      end)
    (export "a" (func $f0)))
;)
