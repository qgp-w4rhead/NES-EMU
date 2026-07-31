"""nes_core — Cython NES emulator core (M11).

Subprocess entry point: ``python -m nes_core`` runs the M3 binary protocol
loop over stdin/stdout. The actual emulation lives in the compiled extension
module ``nes_core._core`` (Cython -> C -> native .pyd).
"""

from nes_core._core import Emulator

__all__ = ["Emulator"]
