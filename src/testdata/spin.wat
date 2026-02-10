(component
  (core module $m
    (func (export "spin")
      (loop (br 0))
    )
  )
  (core instance $i (instantiate $m))
  (func (export "spin") (canon lift (core func $i "spin")))
)
