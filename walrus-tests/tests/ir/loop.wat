(module
  (type (;0;) (func))
  (func (;0;) (type 0)
    loop
    end)
  (export "inf_loop" (func 0)))

;; CHECK: func {
;; NEXT:    ;; function entry
;; NEXT:    block_1():
;; NEXT:      (br block_3 ())
;; NEXT:  
;; NEXT:    ;; loop
;; NEXT:    block_3():
;; NEXT:      (br block_3 ())
;; NEXT:  }
