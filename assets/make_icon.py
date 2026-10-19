"""生成 MemBase 品牌图标：assets/membase.ico（多尺寸 DIB）+ assets/icon_256.rgba（窗口图标用）"""
import struct, os

BG_TOP = (14, 165, 233)     # #0EA5E9
BG_BOT = (3, 105, 161)      # #0369A1
BAR = (255, 255, 255)


def inside_rounded(x, y, size, r):
    if x < r and y < r:
        return (x - r) ** 2 + (y - r) ** 2 <= r * r
    if x > size - 1 - r and y < r:
        return (x - (size - 1 - r)) ** 2 + (y - r) ** 2 <= r * r
    if x < r and y > size - 1 - r:
        return (x - r) ** 2 + (y - (size - 1 - r)) ** 2 <= r * r
    if x > size - 1 - r and y > size - 1 - r:
        return (x - (size - 1 - r)) ** 2 + (y - (size - 1 - r)) ** 2 <= r * r
    return True


def render(size):
    r = max(2, int(size * 0.22))
    px = [[(0, 0, 0, 0) for _ in range(size)] for _ in range(size)]

    # 背景：竖直渐变 + 圆角
    for y in range(size):
        t = y / max(1, size - 1)
        c = tuple(int(BG_TOP[i] + (BG_BOT[i] - BG_TOP[i]) * t) for i in range(3))
        for x in range(size):
            if inside_rounded(x, y, size, r):
                px[y][x] = (c[0], c[1], c[2], 255)

    # 三根递增的柱子（chart 造型），底部对齐
    base_y = int(size * 0.79)
    bar_w = max(1, int(size * 0.14))
    gap = max(1, int(size * 0.09))
    left = int(size * 0.19)
    heights = [0.30, 0.46, 0.62]
    for i, h in enumerate(heights):
        hh = max(2, int(size * h))
        x0 = left + i * (bar_w + gap)
        x1 = min(size - 1, x0 + bar_w - 1)
        y0 = base_y - hh
        for y in range(max(0, y0), base_y):
            for x in range(x0, x1 + 1):
                if 0 <= x < size and inside_rounded(x, y, size, r):
                    px[y][x] = (BAR[0], BAR[1], BAR[2], 255)

    return px


def to_rgba_bytes(px, size):
    out = bytearray()
    for y in range(size):
        for x in range(size):
            r, g, b, a = px[y][x]
            out += bytes((r, g, b, a))
    return bytes(out)


def to_dib(px, size):
    """ICO 里的 DIB：BITMAPINFOHEADER + 自底向上的 BGRA + AND 掩码"""
    hdr = struct.pack(
        "<IiiHHIIiiII",
        40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0
    )
    body = bytearray()
    for y in range(size - 1, -1, -1):
        for x in range(size):
            r, g, b, a = px[y][x]
            body += bytes((b, g, r, a))
    mask_row = ((size + 31) // 32) * 4
    mask = b"\x00" * (mask_row * size)
    return hdr + bytes(body) + mask


def build_ico(sizes, path):
    images = []
    for s in sizes:
        px = render(s)
        images.append((s, to_dib(px, s)))

    out = bytearray()
    out += struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for s, data in images:
        dim = 0 if s >= 256 else s
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for _, data in images:
        out += data

    with open(path, "wb") as f:
        f.write(out)


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    os.makedirs(here, exist_ok=True)
    ico = os.path.join(here, "membase.ico")
    build_ico([256, 64, 48, 32, 16], ico)
    px = render(256)
    with open(os.path.join(here, "icon_256.rgba"), "wb") as f:
        f.write(to_rgba_bytes(px, 256))
    print("ico:", os.path.getsize(ico), "bytes")
    print("rgba:", os.path.getsize(os.path.join(here, "icon_256.rgba")), "bytes")
