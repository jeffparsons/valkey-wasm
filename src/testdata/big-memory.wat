(component
  (core module $m
    (memory (export "mem") 512)
    (func (export "noop"))
  )
  (core instance $i (instantiate $m))
  (func (export "noop") (canon lift (core func $i "noop")))
)
