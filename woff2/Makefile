# Link the woff2 decoder into a Vite-friendly ESM module + a standalone .wasm.
#
# Run inside the Dockerfile's emsdk image (see build.sh) — it needs emcc and the
# static libs baked into /src/build. Differences from wawoff2's Makefile, all in
# service of a clean Vite build:
#   - no -s SINGLE_FILE=1  → emit decompress.wasm as a separate asset (Vite hashes
#                            it via `?url`; the browser stream-compiles it)
#   - -s MODULARIZE=1 -s EXPORT_ES6=1  → default-export an ESM factory (no CJS,
#                            no `module.exports`)
#   - -s ENVIRONMENT=web,worker  → drop the Node branches (require('fs'),
#                            __dirname, process.argv) that Rollup chokes on

BINDING_DIR ?= /src/wawoff2
OUT_DIR     ?= $(BINDING_DIR)/build

CARGS = --bind -O3 \
  -s MODULARIZE=1 \
  -s EXPORT_ES6=1 \
  -s ENVIRONMENT=web,worker \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s EXPORT_NAME=createWoff2Decoder

build:
	mkdir -p $(OUT_DIR)
	emcc $(CARGS) -I/src/woff2/include/ \
	  -o $(OUT_DIR)/decompress.mjs \
	  $(BINDING_DIR)/decompress_binding.cc \
	  /src/build/woff2-wasm/libwoff2dec.a \
	  /src/build/woff2-wasm/libwoff2common.a \
	  /src/build/brotli-wasm/libbrotlidec.a \
	  /src/build/brotli-wasm/libbrotlicommon.a

.PHONY: build
