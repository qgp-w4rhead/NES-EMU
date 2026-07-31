"""Build script for the Cython NES core (M11).

Compiles nes_core/_core.pyx to a native extension module (.pyd) with -O3.
Run:  python setup.py build_ext --inplace
"""
from setuptools import setup, Extension
from Cython.Build import cythonize

extra_compile_args = ["/O3", "/utf-8"]
extra_link_args = []

extensions = [
    Extension(
        "nes_core._core",
        ["nes_core/_core.pyx"],
        extra_compile_args=extra_compile_args,
        extra_link_args=extra_link_args,
    ),
]

setup(
    name="nes_core",
    version="0.1.0",
    description="Cython NES emulator core (M11) speaking the M3 subprocess protocol.",
    packages=["nes_core"],
    ext_modules=cythonize(
        extensions,
        compiler_directives={
            "language_level": "3",
            "boundscheck": False,
            "wraparound": False,
            "initializedcheck": False,
            "cdivision": True,
            "embedsignature": True,
        },
    ),
    zip_safe=False,
)
