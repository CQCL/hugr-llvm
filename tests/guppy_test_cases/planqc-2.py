#!/usr/bin/env python3


@guppy
def teleport(
  src: Qubit, tgt: Qubit
) -> Qubit:
  # Entangle qubits with ancilla
  tmp, tgt = cx(h(Qubit()), tgt)
  src, tmp = cx(src, tmp)
  # Apply classical corrections
  if measure(h(src)):
    tgt = z(tgt)
  if measure(tmp):
    tgt = x(tgt)
  return tgt

@guppy
def main() -> bool:
  q1,q2 = Qubit(), Qubit() # TODO initialise into some interesting state
  return measure(q1,q2)
