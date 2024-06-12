from guppylang.decorator import guppy
from guppylang.module import GuppyModule
from guppylang.prelude import quantum
from guppylang.prelude.builtins import py
from guppylang.prelude.quantum import measure, phased_x, qubit

mod = GuppyModule("main")
mod.load(quantum)

@guppy(mod)
def rx(q: qubit, a: float) -> qubit:
  # Implement Rx via Rz rotation
  return h(rz(h(q), a))


@guppy(mod)
def main() -> bool:
  q = qubit()
  r = rx(q,1.5)
  return measure(r)

if __name__ == "__main__":
    print(mod.compile().serialize())

# import math

# from guppylang.decorator import guppy
# from guppylang.module import GuppyModule
# from guppylang.prelude import quantum
# from guppylang.prelude.builtins import py
# from guppylang.prelude.quantum import measure, phased_x, qubit

# module = GuppyModule("test")
# module.load(quantum)


# @guppy(module)
# def main(q: qubit) -> bool:
#     q = phased_x(q, py(math.pi * 2), 0.0)
#     return measure(q)


# hugr = module.compile()
# print(hugr.to_raw().to_json())
