from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


SOURCE_IMAGE = Path(
    "/Users/chuanpeng/Library/Containers/com.tencent.xinWeChat/Data/Documents/"
    "xwechat_files/wxid_8k8it2sd0rp612_2393/temp/RWTemp/2026-04/"
    "83cfb3b5b629bd84046f82d25b165edb/253fe67370fa04ad79537d71ec01f342.png"
)
OUTPUT_DIR = Path("/Users/chuanpeng/Documents/rkb-lossless-process/release/icon-drafts")
CANVAS_SIZE = 1024
BACKGROUND = (239, 232, 220, 255)
GLYPH = (18, 14, 13, 255)
APP_BACKGROUND = (28, 28, 31, 255)
APP_GLYPH = (242, 243, 245, 255)


def smooth_alpha(alpha: Image.Image) -> Image.Image:
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("No visible glyph found in alpha channel.")

    glyph = alpha.crop(bbox)
    upscale = 4
    large = glyph.resize(
        (glyph.width * upscale, glyph.height * upscale),
        resample=Image.Resampling.BICUBIC,
    )
    large = large.filter(ImageFilter.GaussianBlur(1.2))
    large = large.point(lambda p: 255 if p > 84 else 0 if p < 12 else p)
    smoothed = large.resize(glyph.size, resample=Image.Resampling.LANCZOS)

    canvas = Image.new("L", alpha.size, 0)
    canvas.paste(smoothed, bbox[:2])
    return canvas


def fit_alpha(alpha: Image.Image, target_ratio: float = 0.58) -> Image.Image:
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


def rounded_background(size: int, radius: int = 228) -> Image.Image:
    return rounded_background_with_color(size, BACKGROUND, radius)


def rounded_background_with_color(
    size: int,
    color: tuple[int, int, int, int],
    radius: int = 228,
) -> Image.Image:
    base = Image.new("RGBA", (size, size), color)
    rounded = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(rounded)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    base.putalpha(rounded)
    return base


def main() -> None:
    source = Image.open(SOURCE_IMAGE).convert("RGBA")
    alpha = source.getchannel("A")
    alpha = smooth_alpha(alpha)
    alpha = fit_alpha(alpha)

    bg = rounded_background(CANVAS_SIZE)
    glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), GLYPH)
    glyph.putalpha(alpha)
    final = Image.alpha_composite(bg, glyph)

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    final.save(OUTPUT_DIR / "app-icon-flat-1024.png")

    app_bg = rounded_background_with_color(CANVAS_SIZE, APP_BACKGROUND)
    app_glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), APP_GLYPH)
    app_glyph.putalpha(alpha)
    app_final = Image.alpha_composite(app_bg, app_glyph)
    app_final.save(OUTPUT_DIR / "app-icon-flat-app-tone-1024.png")

    transparent = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    clean_glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), GLYPH)
    clean_glyph.putalpha(alpha)
    transparent = Image.alpha_composite(transparent, clean_glyph)
    transparent.save(OUTPUT_DIR / "glyph-flat-smooth-1024.png")

    app_transparent = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    app_clean_glyph = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), APP_GLYPH)
    app_clean_glyph.putalpha(alpha)
    app_transparent = Image.alpha_composite(app_transparent, app_clean_glyph)
    app_transparent.save(OUTPUT_DIR / "glyph-flat-app-tone-1024.png")


if __name__ == "__main__":
    main()
