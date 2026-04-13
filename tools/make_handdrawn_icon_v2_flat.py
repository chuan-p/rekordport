from __future__ import annotations

from argparse import ArgumentParser
from collections import deque
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


DEFAULT_OUTPUT_DIR = Path(__file__).resolve().parents[1] / "release" / "icon-drafts"
CANVAS_SIZE = 1024
APP_BACKGROUND = (28, 28, 31, 255)
APP_GLYPH = (242, 243, 245, 255)
LIGHT_BACKGROUND = (246, 246, 243, 255)
LIGHT_GLYPH = (95, 96, 99, 255)


def parse_args() -> tuple[Path, Path]:
    parser = ArgumentParser(description="Generate refined hand-drawn flat icon drafts.")
    parser.add_argument("source_image", type=Path, help="Path to the source image.")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help="Directory where the generated assets will be written.",
    )
    args = parser.parse_args()
    return args.source_image.expanduser().resolve(), args.output_dir.expanduser().resolve()


def extract_mask(image: Image.Image, threshold: int = 84) -> Image.Image:
    gray = image.convert("L")
    width, height = gray.size
    pixels = gray.load()
    visited = bytearray(width * height)
    chosen_points: list[tuple[int, int]] = []

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

            if len(points) < 700:
                continue
            if max_y > 1000:
                continue
            chosen_points.extend(points)

    if not chosen_points:
        raise RuntimeError("Failed to isolate the drawing from the photo.")

    xs = [x for x, _ in chosen_points]
    ys = [y for _, y in chosen_points]
    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)

    mask = Image.new("L", (width, height), 0)
    mask_pixels = mask.load()
    for px, py in chosen_points:
        mask_pixels[px, py] = 255

    pad_x = max(12, int((max_x - min_x) * 0.16))
    pad_y = max(12, int((max_y - min_y) * 0.18))
    cropped = mask.crop(
        (
            max(0, min_x - pad_x),
            max(0, min_y - pad_y),
            min(width, max_x + pad_x),
            min(height, max_y + pad_y),
        )
    )
    return cropped


def smooth_alpha(alpha: Image.Image) -> Image.Image:
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Mask is empty.")

    glyph = alpha.crop(bbox)
    upscale = 4
    large = glyph.resize(
        (glyph.width * upscale, glyph.height * upscale),
        resample=Image.Resampling.BICUBIC,
    )
    large = large.filter(ImageFilter.GaussianBlur(1.2))
    large = large.point(lambda p: 255 if p > 86 else 0 if p < 8 else p)
    smoothed = large.resize(glyph.size, resample=Image.Resampling.LANCZOS)

    canvas = Image.new("L", alpha.size, 0)
    canvas.paste(smoothed, bbox[:2])
    return canvas


def fit_alpha(alpha: Image.Image, target_ratio: float = 0.44) -> Image.Image:
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Alpha is empty after smoothing.")

    glyph = alpha.crop(bbox)
    scale = CANVAS_SIZE * target_ratio / max(glyph.width, glyph.height)
    resized = glyph.resize(
        (max(1, round(glyph.width * scale)), max(1, round(glyph.height * scale))),
        resample=Image.Resampling.LANCZOS,
    )

    canvas = Image.new("L", (CANVAS_SIZE, CANVAS_SIZE), 0)
    offset = ((CANVAS_SIZE - resized.width) // 2, (CANVAS_SIZE - resized.height) // 2)
    canvas.paste(resized, offset)
    return canvas


def rounded_background(size: int, color: tuple[int, int, int, int], radius: int = 228) -> Image.Image:
    base = Image.new("RGBA", (size, size), color)
    rounded = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(rounded)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    base.putalpha(rounded)
    return base


def main() -> None:
    source_image, output_dir = parse_args()
    image = Image.open(source_image).convert("RGB")
    alpha = extract_mask(image)
    alpha = smooth_alpha(alpha)
    alpha = fit_alpha(alpha)

    output_dir.mkdir(parents=True, exist_ok=True)

    bg = rounded_background(CANVAS_SIZE, APP_BACKGROUND)
    glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), APP_GLYPH)
    glyph.putalpha(alpha)
    final = Image.alpha_composite(bg, glyph)
    final.save(output_dir / "app-icon-flat-app-tone-v2-1024.png")

    light_bg = rounded_background(CANVAS_SIZE, LIGHT_BACKGROUND)
    light_glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), LIGHT_GLYPH)
    light_glyph.putalpha(alpha)
    light_final = Image.alpha_composite(light_bg, light_glyph)
    light_final.save(output_dir / "app-icon-flat-light-v2-1024.png")

    transparent = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    plain = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), APP_GLYPH)
    plain.putalpha(alpha)
    transparent = Image.alpha_composite(transparent, plain)
    transparent.save(output_dir / "glyph-flat-app-tone-v2-1024.png")

    light_transparent = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    light_plain = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), LIGHT_GLYPH)
    light_plain.putalpha(alpha)
    light_transparent = Image.alpha_composite(light_transparent, light_plain)
    light_transparent.save(output_dir / "glyph-flat-light-v2-1024.png")


if __name__ == "__main__":
    main()
