# -*- coding: utf-8 -*-
"""把 GUI 自检产出的 32 位 BMP 转成 PNG（纯标准库，不额外装依赖）。"""
import struct
import sys
import zlib


def bmp_to_png(src, dst):
    d = open(src, 'rb').read()
    off = struct.unpack_from('<I', d, 10)[0]
    w = struct.unpack_from('<i', d, 18)[0]
    h = struct.unpack_from('<i', d, 22)[0]
    bpp = struct.unpack_from('<H', d, 28)[0]
    if bpp != 32:
        raise SystemExit('only 32bpp bmp supported')
    top_down = h < 0
    h = abs(h)
    stride = w * 4
    rows = []
    for y in range(h):
        sy = y if top_down else h - 1 - y
        base = off + sy * stride
        row = d[base:base + stride]
        rgb = bytearray(w * 3 + 1)
        for x in range(w):
            b, g, r, _ = row[x * 4:x * 4 + 4]
            rgb[1 + x * 3] = r
            rgb[2 + x * 3] = g
            rgb[3 + x * 3] = b
        rows.append(bytes(rgb))

    def chunk(tag, data):
        return (struct.pack('>I', len(data)) + tag + data
                + struct.pack('>I', zlib.crc32(tag + data) & 0xFFFFFFFF))

    png = b'\x89PNG\r\n\x1a\n'
    png += chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(b''.join(rows), 6))
    png += chunk(b'IEND', b'')
    open(dst, 'wb').write(png)
    print('%s  %dx%d' % (dst, w, h))


if __name__ == '__main__':
    bmp_to_png(sys.argv[1], sys.argv[2])
