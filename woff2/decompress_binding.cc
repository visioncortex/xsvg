/*
   WOFF2 → SFNT (TTF) decompression, exposed to JS via embind.

   Adapted from fontello/wawoff2 (MIT). We build only the decoder — the web
   app just needs woff2 → ttf so opentype.js can parse Google Fonts' woff2.

   Distributed under MIT license.
   See file LICENSE for detail or copy at https://opensource.org/licenses/MIT
*/

#include <woff2/decode.h>
#include <emscripten/bind.h>

emscripten::val decompress(std::string input) {
  const uint8_t* raw_input = reinterpret_cast<const uint8_t*>(input.data());

  std::string output(
    std::min(woff2::ComputeWOFF2FinalSize(raw_input, input.size()), woff2::kDefaultMaxSize),
    0);

  woff2::WOFF2StringOut out(&output);

  if (!woff2::ConvertWOFF2ToTTF(raw_input, input.size(), &out)) {
    return emscripten::val(false);
  }

  return emscripten::val(
    emscripten::typed_memory_view(output.size(), reinterpret_cast<unsigned const char*>(output.data()))
  );
}

EMSCRIPTEN_BINDINGS(xsvg_woff2) {
  emscripten::function("decompress", &decompress);
}
