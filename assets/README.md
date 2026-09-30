# Fonts

The `font_*.bmp` files are the M8's bitmap font atlases, extracted verbatim
from [m8c](https://github.com/laamaa/m8c) (`src/fonts/font1.h` … `font5.h`),
which is MIT licensed:

> Copyright (c) 2021 Jonne Kokkonen
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the above copyright notice and this permission
> notice being included in all copies or substantial portions of the Software.

Each atlas is a 1-bit BMP holding 94 glyphs in a single row, starting at
ASCII 33 (`!`). Glyph width is the image width divided by 94.

| File                 | Glyph cell | Used by                        |
|----------------------|-----------:|--------------------------------|
| `font_v1_small.bmp`  |      5 x 7 | Model:01, font mode 0          |
| `font_v1_large.bmp`  |      8 x 9 | Model:01, font mode 1          |
| `font_v2_small.bmp`  |      9 x 9 | Model:02, font mode 0          |
| `font_v2_large.bmp`  |    10 x 10 | Model:02, font mode 1          |
| `font_v2_huge.bmp`   |    12 x 12 | Model:02, font mode 2          |
