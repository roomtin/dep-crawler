# Mock C Project: "Orchard"

This project is designed to **stress-test a dependency crawler**. It leans into tricky `#include` cases:
- Angle vs quoted includes
- Nested directories and relative `..` includes
- Conditional includes by platform
- Macro-expanded `#include` of a generated header
- A **cycle** between headers (guarded) to verify your graph represents cycles
- Third-party subtree with its own headers
- Plugins that reach back into `include/` via `..`

## Layout
- `include/`: public headers
- `include/platform/`: platform-specific headers (posix/win)
- `include/util/`: utility headers
- `include/model/`: intentionally cyclic pair (`node.h` <-> `edge.h`)
- `src/`: implementation files
- `plugins/`: sample plugin using relative include into `include/`
- `generated/`: simulates a generated header (referenced via macro include)
- `third_party/libfoo/`: faux external library
- `build/`: output folder (empty)

Your crawler should follow **only** the includes it can see in-source. If you later support `-I` flags, try these:
```
-Iinclude -Iplugins -Igenerated -Ithird_party/libfoo/include
```

## Interesting edges your crawler should detect

- `src/main.c` -> `include/common.h`, `include/graph.h`
- `include/common.h` -> `include/config.h`, `include/util/math.h`, `generated/autogen.h`
- `include/config.h` -> `include/platform/posix.h` or `include/platform/win.h` (conditional)
- `include/util/math.h` -> `include/util/number.h`
- `include/graph.h` -> `include/model/node.h`
- `include/model/node.h` <-> `include/model/edge.h` (cycle via guarded headers)
- `src/graph.c` -> `include/graph.h`, `include/util/math.h`
- `src/platform.c` -> `include/config.h`
- `plugins/plugin.h` -> `include/common.h` (via `../include/common.h`)
- `plugins/plugin.c` -> `plugins/plugin.h`
- `third_party/libfoo/src/foo.c` -> `third_party/libfoo/include/foo/foo.h` -> `third_party/libfoo/include/foo/bar.h`

## Build (optional)
This compiles a small program. The cycle is guarded by include guards and forward decls.
```sh
make
./build/orchard
```

