(module
  (type (;0;) (func (param i32) (result i32)))
  (func $inc (type 0) (param i32) (result i32)
    get_local 0
    i32.const 1
    i32.add)
  (table (;0;) 1 1 anyfunc)
  (memory (;0;) 16))
