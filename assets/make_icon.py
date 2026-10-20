"""从品牌源图生成 MemBase 图标资产。

输入：assets/membase_src.png（方形 logo，白底 + 圆角）
输出：
  assets/membase.ico      多尺寸 32 位 DIB，供 build.rs / winres 嵌入 exe
  assets/icon_256.rgba    256x256 原始 RGBA，供 egui 窗口图标（include_bytes!）
  assets/membase.png      256x256 带透明通道的 PNG，README / 商店页面用

用法（仅开发期需要，构建不需要 Pillow）：
  python assets/make_icon.py
"""
import os
import struct

from PIL import Image

SIZES = [256, 128, 64, 48, 32, 24, 16]
MASTER = 256
WHITE_CUTOFF = 12   # 与纯白的差值超过该值视为“图形内部”
MASK_EDGE = 256     # 圆角轮廓的基准边长，下面的 CORNER_PROFILE 就是在 256 边上量的

# 圆角轮廓：取自 Windows 桌面图标「息屏助手.exe」的 256 帧 alpha 通道（四角完全对称）。
# CORNER_PROFILE[i] = 第 i 行（从边界往内数）第一个不透明像素的 x，量到第 54 行收到 0。
# 这是一条比正圆弧更“软”的超椭圆曲线：半径约 54/256 ≈ 21%，但收边更平缓。
CORNER_PROFILE = (
    54, 48, 44, 41, 38, 36, 34, 32, 30, 29, 27, 26, 24, 23, 22, 21, 20, 19,
    18, 17, 16, 15, 14, 13, 12, 12, 11, 10, 10, 9, 8, 8, 7, 7, 6, 6, 5, 5,
    4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1,
)
CORNER_DEPTH = len(CORNER_PROFILE) - 1  # 轮廓最后一行的下标，再往里就是直边


def shape_bbox(img, cutoff=WHITE_CUTOFF, frac=0.2):
    """找到圆角方块的实际边界（去掉白边），返回 (x0, y0, x1, y1)，右下开区间。

    按行列统计非白像素数，只认“够长”的行列，避免零散噪点把边界撑大。
    """
    w, h = img.size
    px = img.load()
    col_cnt = [0] * w
    row_cnt = [0] * h
    for y in range(h):
        for x in range(w):
            r, g, b = px[x, y]
            if 255 - min(r, g, b) > cutoff:
                col_cnt[x] += 1
                row_cnt[y] += 1
    min_col = max(1, int(h * frac))
    min_row = max(1, int(w * frac))
    xs = [x for x, c in enumerate(col_cnt) if c >= min_col]
    ys = [y for y, c in enumerate(row_cnt) if c >= min_row]
    if not xs or not ys:
        return 0, 0, w, h
    return min(xs), min(ys), max(xs) + 1, max(ys) + 1


def corner_inset(t):
    """t 为距边界的行数（以 256 边长为基准），返回同一基准下的左边缩进。"""
    if t >= CORNER_DEPTH:
        return 0.0
    i = int(t)
    frac = t - i
    a = CORNER_PROFILE[i]
    b = CORNER_PROFILE[min(i + 1, CORNER_DEPTH)]
    return a + (b - a) * frac


def rounded_mask(size, ss=4):
    """按 CORNER_PROFILE 的超采样圆角遮罩，全画布出血（四边贴边）。"""
    big = size * ss
    scale = big / MASK_EDGE
    mask = Image.new("L", (big, big), 0)
    px = mask.load()
    for y in range(big):
        dy = min(y, big - 1 - y) / scale
        x0 = int(round(corner_inset(dy) * scale))
        x1 = big - x0
        if x1 <= x0:
            continue
        for x in range(x0, x1):
            px[x, y] = 255
    return mask.resize((size, size), Image.LANCZOS)


def render(size, master):
    """master 为已裁掉白边的方形 RGB 图。"""
    mask = rounded_mask(size)
    art = master.resize((size, size), Image.LANCZOS).convert("RGB")
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    art_px = art.load()
    mask_px = mask.load()
    out_px = out.load()
    for y in range(size):
        for x in range(size):
            a = mask_px[x, y]
            if a:
                r, g, b = art_px[x, y]
                out_px[x, y] = (r, g, b, a)
    return out


def to_rgba_bytes(img):
    return img.tobytes()


def to_dib(img):
    """ICO 里的 DIB：BITMAPINFOHEADER + 自底向上 BGRA + AND 掩码"""
    size = img.size[0]
    hdr = struct.pack(
        "<IiiHHIIiiII",
        40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0
    )
    src = img.load()
    body = bytearray()
    for y in range(size - 1, -1, -1):
        for x in range(size):
            r, g, b, a = src[x, y]
            body += bytes((b, g, r, a))

    # AND 掩码：1 = 透明。必须和 alpha 一致。
    # 之前这里写全 0，桌面在走掩码的路径上会忽略 alpha，四角就成了实的直角。
    mask_row = ((size + 31) // 32) * 4
    mask = bytearray(mask_row * size)
    for y in range(size):
        sy = size - 1 - y          # 掩码和位图一样自底向上
        base = y * mask_row
        for x in range(size):
            if src[x, sy][3] < 128:
                mask[base + (x >> 3)] |= 0x80 >> (x & 7)
    return hdr + bytes(body) + bytes(mask)


def build_ico(images, path):
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


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    src_path = os.path.join(here, "membase_src.png")
    src = Image.open(src_path).convert("RGB")

    x0, y0, x1, y1 = shape_bbox(src)
    # 再往里收 1px：源图边缘那一圈是和白底混过的抗锯齿像素，留着会有一圈白边
    side = max(x1 - x0, y1 - y0) - 2
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    box = (
        int(round(cx - side / 2)), int(round(cy - side / 2)),
        int(round(cx + side / 2)), int(round(cy + side / 2)),
    )
    master = src.crop(box)

    rendered = [(s, render(s, master)) for s in SIZES]
    ico = os.path.join(here, "membase.ico")
    build_ico([(s, to_dib(img)) for s, img in rendered], ico)

    big = dict(rendered)[MASTER]
    with open(os.path.join(here, "icon_256.rgba"), "wb") as f:
        f.write(to_rgba_bytes(big))
    big.save(os.path.join(here, "membase.png"))

    print("shape bbox:", box)
    print("corner: 轮廓深 %d/%d, 首行缩进 %.1f%%"
          % (CORNER_DEPTH, MASK_EDGE, CORNER_PROFILE[0] / MASK_EDGE * 100))
    print("ico:", os.path.getsize(ico), "bytes /", len(rendered), "frames")
    print("rgba:", os.path.getsize(os.path.join(here, "icon_256.rgba")), "bytes")


if __name__ == "__main__":
    main()
