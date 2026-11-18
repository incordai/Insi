# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Managed-memory retention controls independent of process RSS high-water marks."""
import gc
import tracemalloc
import unittest


# Compare live traced allocations after collection rather than allocator-reserved memory.
def retained_bytes(leaking):
    tracemalloc.start()
    try:
        before = tracemalloc.get_traced_memory()[0]
        retained = []
        for _ in range(2048):
            block = bytearray(4096)
            if leaking:
                retained.append(block)
        del block
        gc.collect()
        return tracemalloc.get_traced_memory()[0] - before
    finally:
        tracemalloc.stop()


# Both controls must work before any memory detection capability is claimed.
class MemoryTests(unittest.TestCase):
    def test_retained_allocations_are_distinguished_from_released_allocations(self):
        self.assertGreater(retained_bytes(True), 8 * 1024 * 1024)
        self.assertLess(retained_bytes(False), 128 * 1024)


if __name__ == "__main__":
    unittest.main()
