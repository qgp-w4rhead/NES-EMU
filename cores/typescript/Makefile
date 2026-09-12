# TypeScript NES core build (M10)
# Produces dist/main.js, loaded by the harness via `node dist/main.js`.

TSFLAGS =
SRC = src

.PHONY: all clean test

all:
	tsc $(TSFLAGS)

test:
	tsc -p tsconfig.test.json
	node dist/test/run_tests.js

clean:
	@node -e "require('fs').rmSync('dist',{recursive:true,force:true})"
