(module
  (table 1 anyfunc)
  (elem (i32.const 0) 0)
  (func $f (param i64) (result i32)
    local.get 0
    i64.eqz)

  (export "a" (table 0)))

(; CHECK-ALL:
  (module
    (type (;0;) (func (param i32 i32) (result i32)))
    (func $f (type 0) (param i32 i32) (result i32)
      (local $temp_low3 i32)
      block (result i32)  ;; label = @1
        local.get 0
        local.set $temp_low3
        local.get 1
      end
      i32.eqz
      local.get $temp_low3
      i32.eqz
      i32.and)
    (table (;0;) 1 anyfunc)
    (export "a" (table 0))
    (elem (;0;) (i32.const 0) $f))
;)
