(module
  (memory (export "memory") 1)
  (func (export "optimize") (param $len i32) (result i32)
    local.get $len))
