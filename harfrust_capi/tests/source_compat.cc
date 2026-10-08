// Chromium uses these macros in bool aggregate initializers and passes its
// Latin-1 byte spans without a cast. Keep the HarfBuzz source types intact.
#include "hr-hb.h"
#include <type_traits>

static_assert(HB_DIRECTION_IS_VALID(HB_DIRECTION_LTR), "valid direction");
static_assert(!HB_DIRECTION_IS_VALID(HB_DIRECTION_INVALID), "invalid direction");
static_assert(HB_DIRECTION_IS_HORIZONTAL(HB_DIRECTION_RTL), "horizontal");
static_assert(HB_DIRECTION_IS_VERTICAL(HB_DIRECTION_TTB), "vertical");
static_assert(HB_DIRECTION_IS_FORWARD(HB_DIRECTION_TTB), "forward");
static_assert(HB_DIRECTION_IS_BACKWARD(HB_DIRECTION_BTT), "backward");
static_assert(HB_DIRECTION_REVERSE(HB_DIRECTION_LTR) == HB_DIRECTION_RTL, "reverse");
static_assert(std::is_same<decltype(HB_DIRECTION_IS_HORIZONTAL(HB_DIRECTION_LTR)),
                           bool>::value, "direction predicates return bool");

using AddLatin1 = void (*)(hb_buffer_t *, const uint8_t *, int, unsigned int, int);
static_assert(std::is_same<decltype(&hb_buffer_add_latin1), AddLatin1>::value,
              "Latin-1 accepts unsigned bytes");

struct Orientation {
  bool horizontal;
};

Orientation orientation(hb_direction_t direction) {
  return {HB_DIRECTION_IS_HORIZONTAL(direction)};
}
