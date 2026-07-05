# Reproducible emscripten toolchain for the woff2 → sfnt decoder.
#
# Mirrors fontello/wawoff2's build (google/woff2 + brotli via emcc) but pins the
# brotli submodule to the woff2 commit and skips the encoder + native builds —
# the web app only decompresses. The actual link is driven by ./Makefile at
# `docker run` time (see build.sh); this image just supplies emcc + the static
# libs it links against.
#
# amd64: the emsdk 2.0.34 tag is amd64-only, so on Apple Silicon it runs under
# emulation (build.sh passes --platform=linux/amd64 to `docker build`). It's a
# one-time build; the artifacts are committed to the repo.
FROM emscripten/emsdk:2.0.34

# google/woff2 pinned to the same commit wawoff2 ships, with its brotli
# submodule resolved deterministically (git checkout alone leaves brotli at the
# clone's HEAD; the explicit submodule update pins it to what this woff2 expects).
RUN git clone https://github.com/google/woff2.git /src/woff2 && \
    cd /src/woff2 && \
    git checkout a0d0ed7da27b708c0a4e96ad7a998bddc933c06e && \
    git submodule update --init --recursive

# brotli → wasm static libs (enc/dec/common). We only link dec + common, but
# woff2's cmake wants both enc and dec paths defined to configure.
RUN mkdir -p /src/build/brotli-wasm && cd /src/build/brotli-wasm && \
    emcmake cmake /src/woff2/brotli -DCMAKE_BUILD_TYPE=Release && \
    emmake make -j4

# woff2 → wasm decoder static libs (libwoff2dec.a + libwoff2common.a).
RUN mkdir -p /src/build/woff2-wasm && cd /src/build/woff2-wasm && \
    emcmake cmake /src/woff2 \
      -DCMAKE_BUILD_TYPE=Release \
      -DNOISY_LOGGING=OFF \
      -DBROTLIENC_INCLUDE_DIRS=/src/woff2/brotli/c/include/ \
      -DBROTLIDEC_INCLUDE_DIRS=/src/woff2/brotli/c/include/ \
      -DBROTLIENC_LIBRARIES=/src/build/brotli-wasm/libbrotlienc.a \
      -DBROTLIDEC_LIBRARIES=/src/build/brotli-wasm/libbrotlidec.a && \
    emmake make -j4 woff2common woff2dec

# Pre-warm the emscripten cache (libc++/embind) with our exact link flags, into
# a world-writable dir, so the `docker run` link succeeds regardless of the uid
# the container runs as.
ENV EM_CACHE=/emcache
RUN mkdir -p /emcache && chmod 777 /emcache && \
    printf '#include <emscripten/bind.h>\nint f(){return 0;}\nEMSCRIPTEN_BINDINGS(w){emscripten::function("f",&f);}\n' > /tmp/warm.cc && \
    emcc --bind -O3 -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=web,worker -sALLOW_MEMORY_GROWTH=1 /tmp/warm.cc -o /tmp/warm.mjs && \
    chmod -R 777 /emcache
