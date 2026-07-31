# TypeScript NES core build (M10)
# Produces dist/main.js, loaded by the harness via `node dist/main.js`.

TSFLAGS =
SRC = src

.PHONY: all clean

all:
	tsc $(TSFLAGS)

clean:
	@node -e "require('fs').rmSync('dist',{recursive:true,force:true})"
