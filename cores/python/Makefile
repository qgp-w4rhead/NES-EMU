# Makefile for the Cython NES core (M11)
# Build: make build
# Test:  make test
# Bench: make bench (requires harness.exe in ../../bench/)
#
# NOTE: On Windows, run bench/fbcompare from PowerShell with:
#   $env:PYTHONPATH = "cores\python"; .\bench\harness.exe --subprocess-bench "python -m nes_core"

PYTHON = python
SETUP = setup.py

.PHONY: build test bench fbcompare clean

build:
	$(PYTHON) $(SETUP) build_ext --inplace

test:
	$(PYTHON) test/run_tests.py

bench:
	@echo "Run from PowerShell:"
	@echo '  $$env:PYTHONPATH = "."; ../../bench/harness.exe --subprocess-bench "python -m nes_core" --subprocess-name "python-cython"'

fbcompare:
	@echo "Run from PowerShell:"
	@echo '  $$env:PYTHONPATH = "."; ../../bench/fb_compare_sub.exe ../../cores/c/nes_core_c.dll "python -m nes_core" 20'

clean:
	-rmdir /s /q build 2>nul || rm -rf build
	-del nes_core\_core.c 2>nul || rm -f nes_core/_core.c
	-del nes_core\*.pyd 2>nul || rm -f nes_core/*.pyd
	-rmdir /s /q nes_core\__pycache__ 2>nul || rm -rf nes_core/__pycache__
	-rmdir /s /q test\__pycache__ 2>nul || rm -rf test/__pycache__
