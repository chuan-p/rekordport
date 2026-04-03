from __future__ import annotations

from collections import deque
from pathlib import Path
import math
import random

from PIL import Image, ImageChops, ImageDraw, ImageFilter


SOURCE_IMAGE = Path(
    "/Users/chuanpeng/Library/Containers/com.tencent.xinWeChat/Data/Documents/"
    "xwechat_files/wxid_8k8it2sd0rp612_2393/temp/RWTemp/2026-04/"
    "83cfb3b5b629bd84046f82d25b165edb/8df212ab5d32ce5b52f28eb6247f57a0.jpg"
)
OUTPUT_DIR = Path("/Users/chuanpeng/Documents/rkb-lossless-process/release/icon-drafts")


def extract_main_shape_mask(image: Image.Image, threshold: int = 72) -> Image.Image:
    gray = image.convert("L")
    width, height = gray.size
    pixels = gray.load()
    visited = bytearray(width * height)
    components: list[tuple[int, tuple[int, int, int, int], list[tuple[int, int]]]] = []

    for y in range(height):
        for x in range(width):
            idx = y * width + x
            if visited[idx] or pixels[x, y] >= threshold:
                continue

            queue = deque([(x, y)])
            visited[idx] = 1
            points: list[tuple[int, int]] = []
            min_x = max_x = x
            min_y = max_y = y

            while queue:
                cx, cy = queue.popleft()
                points.append((cx, cy))
                min_x = min(min_x, cx)
                min_y = min(min_y, cy)
                max_x = max(max_x, cx)
                max_y = max(max_y, cy)

                for nx, ny in ((cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)):
                    if 0 <= nx < width and 0 <= ny < height:
                        n_idx = ny * width + nx
                        if not visited[n_idx] and pixels[nx, ny] < threshold:
                            visited[n_idx] = 1
                            queue.append((nx, ny))

            components.append((len(points), (min_x, min_y, max_x, max_y), points))

    components.sort(key=lambda item: item[0], reverse=True)
    if not components:
        raise RuntimeError("No dark shape found in the source image.")

    _, bbox, points = components[0]
    min_x, min_y, max_x, max_y = bbox
    mask = Image.new("L", (width, height), 0)
    mask_pixels = mask.load()
    for px, py in points:
        mask_pixels[px, py] = 255

    pad_x = int((max_x - min_x) * 0.14)
    pad_y = int((max_y - min_y) * 0.12)
    crop_box = (
        max(0, min_x - pad_x),
        max(0, min_y - pad_y),
        min(width, max_x + pad_x),
        min(height, max_y + pad_y),
    )
    cropped = mask.crop(crop_box)
    cropped = cropped.filter(ImageFilter.MaxFilter(5))
    cropped = cropped.filter(ImageFilter.GaussianBlur(0.8))
    return cropped.point(lambda p: 255 if p > 16 else 0)


def fit_mask(mask: Image.Image, size: int = 1024, target_ratio: float = 0.56) -> Image.Image:
    bbox = mask.getbbox()
    if not bbox:
        raise RuntimeError("Mask is empty after extraction.")

    shape = mask.crop(bbox)
    shape_w, shape_h = shape.size
    scale = size * target_ratio / max(shape_w, shape_h)
    resized = shape.resize(
        (max(1, round(shape_w * scale)), max(1, round(shape_h * scale))),
        resample=Image.Resampling.LANCZOS,
    )
    resized = resized.filter(ImageFilter.MaxFilter(3))

    canvas = Image.new("L", (size, size), 0)
    offset = ((size - resized.width) // 2, (size - resized.height) // 2)
    canvas.paste(resized, offset)
    return canvas


def add_paper_texture(image: Image.Image, seed: int = 7) -> Image.Image:
    rng = random.Random(seed)
    width, height = image.size
    noise = Image.new("L", (width, height))
    values = bytearray(width * height)
    for i in range(width * height):
        values[i] = max(0, min(255, 128 + rng.randint(-24, 24)))
    noise.frombytes(bytes(values))
    noise = noise.filter(ImageFilter.GaussianBlur(0.7))
    texture = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    alpha = ImageChops.multiply(noise, Image.new("L", (width, height), 34))
    texture.putalpha(alpha)
    return Image.alpha_composite(image, texture)


def build_gradient(size: int, top_rgb: tuple[int, int, int], bottom_rgb: tuple[int, int, int]) -> Image.Image:
    gradient = Image.new("RGBA", (size, size))
    px = gradient.load()
    for y in range(size):
        t = y / (size - 1)
        r = round(top_rgb[0] * (1 - t) + bottom_rgb[0] * t)
        g = round(top_rgb[1] * (1 - t) + bottom_rgb[1] * t)
        b = round(top_rgb[2] * (1 - t) + bottom_rgb[2] * t)
        for x in range(size):
            px[x, y] = (r, g, b, 255)
    return gradient


def apply_rounded_mask(image: Image.Image, radius: int = 228) -> Image.Image:
    rounded = Image.new("L", image.size, 0)
    draw = ImageDraw.Draw(rounded)
    draw.rounded_rectangle((0, 0, image.width, image.height), radius=radius, fill=255)
    image.putalpha(rounded)
    return image


def render_paper_icon(mask: Image.Image, out_path: Path) -> None:
    size = mask.width
    base = build_gradient(size, (247, 241, 230), (219, 198, 172))
    base = add_paper_texture(base)

    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    glow_draw = ImageDraw.Draw(glow)
    glow_draw.ellipse((120, 90, size - 120, size - 140), fill=(255, 255, 255, 44))
    glow = glow.filter(ImageFilter.GaussianBlur(48))
    base = Image.alpha_composite(base, glow)

    shadow = Image.new("RGBA", (size, size), (33, 22, 17, 0))
    shadow.putalpha(mask.filter(ImageFilter.GaussianBlur(22)))
    shadow = ImageChops.offset(shadow, 16, 20)
    shadow_alpha = shadow.getchannel("A").point(lambda p: min(110, p))
    shadow.putalpha(shadow_alpha)
    base = Image.alpha_composite(base, shadow)

    highlight_ring = ImageChops.subtract(
        mask.filter(ImageFilter.MaxFilter(9)),
        mask.filter(ImageFilter.MaxFilter(3)),
    ).filter(ImageFilter.GaussianBlur(1.3))
    highlight = Image.new("RGBA", (size, size), (255, 250, 242, 0))
    highlight.putalpha(highlight_ring.point(lambda p: min(52, p)))
    highlight = ImageChops.offset(highlight, -2, -3)
    base = Image.alpha_composite(base, highlight)

    glyph = Image.new("RGBA", (size, size), (20, 16, 15, 0))
    glyph.putalpha(mask)
    base = Image.alpha_composite(base, glyph)

    apply_rounded_mask(base)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    base.save(out_path)


def render_dark_icon(mask: Image.Image, out_path: Path) -> None:
    size = mask.width
    base = build_gradient(size, (25, 20, 18), (56, 41, 33))

    light_bloom = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    bloom_draw = ImageDraw.Draw(light_bloom)
    bloom_draw.ellipse((170, 130, size - 170, size - 170), fill=(255, 186, 128, 36))
    light_bloom = light_bloom.filter(ImageFilter.GaussianBlur(60))
    base = Image.alpha_composite(base, light_bloom)

    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow.putalpha(mask.filter(ImageFilter.GaussianBlur(28)))
    shadow = ImageChops.offset(shadow, 12, 18)
    shadow_alpha = shadow.getchannel("A").point(lambda p: min(150, p))
    shadow.putalpha(shadow_alpha)
    base = Image.alpha_composite(base, shadow)

    glyph = Image.new("RGBA", (size, size), (244, 234, 220, 0))
    glyph.putalpha(mask)
    base = Image.alpha_composite(base, glyph)

    apply_rounded_mask(base)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    base.save(out_path)


def render_clean_icon(mask: Image.Image, out_path: Path) -> None:
    size = mask.width
    base = build_gradient(size, (245, 239, 229), (229, 217, 200))

    vignette = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    vignette_draw = ImageDraw.Draw(vignette)
    vignette_draw.ellipse((86, 70, size - 86, size - 110), fill=(255, 255, 255, 32))
    vignette = vignette.filter(ImageFilter.GaussianBlur(54))
    base = Image.alpha_composite(base, vignette)

    shadow = Image.new("RGBA", (size, size), (49, 35, 27, 0))
    shadow.putalpha(mask.filter(ImageFilter.GaussianBlur(20)))
    shadow = ImageChops.offset(shadow, 14, 18)
    shadow.putalpha(shadow.getchannel("A").point(lambda p: min(96, p)))
    base = Image.alpha_composite(base, shadow)

    glyph = Image.new("RGBA", (size, size), (16, 13, 12, 0))
    glyph.putalpha(mask)
    base = Image.alpha_composite(base, glyph)

    apply_rounded_mask(base)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    base.save(out_path)


def render_transparent_glyph(mask: Image.Image, out_path: Path) -> None:
    size = mask.width
    glyph = Image.new("RGBA", (size, size), (20, 16, 15, 0))
    glyph.putalpha(mask)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    glyph.save(out_path)


def main() -> None:
    image = Image.open(SOURCE_IMAGE).convert("RGB")
    mask = extract_main_shape_mask(image)
    mask = fit_mask(mask, size=1024, target_ratio=0.58)

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    render_clean_icon(mask, OUTPUT_DIR / "app-icon-clean-1024.png")
    render_paper_icon(mask, OUTPUT_DIR / "app-icon-paper-1024.png")
    render_dark_icon(mask, OUTPUT_DIR / "app-icon-dark-1024.png")
    render_transparent_glyph(mask, OUTPUT_DIR / "glyph-transparent-1024.png")


if __name__ == "__main__":
    main()
