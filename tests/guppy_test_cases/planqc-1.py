#!/usr/bin/env python3

@guppy
def rx(q: Qubit, a: float) -> Qubit:
  # Implement Rx via Rz rotation
  return h(rz(h(q), a))


@guppy
def main() -> bool:
  q = Qubit()
  r = rx(q,1.5)
  return measure(r)
