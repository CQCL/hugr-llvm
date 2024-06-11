#!/usr/bin/env python3


@guppy
def rus(q: Qubit, tries: int) -> Qubit:
  for _ in range(tries):
    # Prepare ancillary qubits
    a, b = h(Qubit()), h(Qubit())

    b, a = cx(b, tdg(a))
    if not measure(t(a)):
      # First part failed; try again
      discard(b)
      continue

    q, b = cx(z(t(q)), b)
    if measure(t(b)):
      # Success, we are done
      break

    # Otherwise, apply correction
    q = x(q)

  return q

@guppy
def main() -> bool:
    q = Qubit() # todo initialise into an interesting state
    return measure(rus(q,100))
