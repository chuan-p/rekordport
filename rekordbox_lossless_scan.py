#!/usr/bin/env python3
"""Scan a Rekordbox master.db for FLAC, ALAC, and high-bit-depth WAV/AIFF tracks.

This script reads the encrypted Rekordbox 6 master database through the
`sqlcipher` CLI, so it does not need any Python SQLCipher bindings.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List
from xml.sax.saxutils import escape

DEFAULT_DB_PATH = Path("~/Library/Pioneer/rekordbox/master.db").expanduser()
DEFAULT_KEY = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497"

FILE_TYPE_NAMES = {
    4: "M4A",
    5: "FLAC",
    11: "WAV",
    12: "AIFF",
}


@dataclass(frozen=True)
class Track:
    id: str
    title: str
    artist: str
    file_type: int
    file_type_name: str
    codec_name: str | None
    bit_depth: int | None
    sample_rate: int | None
    bitrate: int | None
    full_path: str

    def to_dict(self) -> dict[str, object]:
        return {
            "id": self.id,
            "title": self.title,
            "artist": self.artist,
            "file_type": self.file_type_name,
            "codec_name": self.codec_name,
            "bit_depth": self.bit_depth,
            "sample_rate": self.sample_rate,
            "bitrate": self.bitrate,
            "full_path": self.full_path,
        }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Find FLAC, ALAC, and WAV/AIFF tracks with bit depth greater than 16 in Rekordbox."
    )
    parser.add_argument(
        "--db",
        type=Path,
        default=DEFAULT_DB_PATH,
        help=f"Path to Rekordbox master.db (default: {DEFAULT_DB_PATH})",
    )
    parser.add_argument(
        "--key",
        default=DEFAULT_KEY,
        help="SQLCipher key for the Rekordbox database.",
    )
    parser.add_argument(
        "--min-bit-depth",
        type=int,
        default=16,
        help="Minimum bit depth threshold for WAV/AIFF tracks.",
    )
    parser.add_argument(
        "--format",
        choices=("table", "csv", "json", "xlsx"),
        default="table",
        help="Output format.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Write the result to a file instead of standard output.",
    )
    parser.add_argument(
        "--sqlcipher",
        default="sqlcipher",
        help="Path to the sqlcipher executable.",
    )
    parser.add_argument(
        "--include-sampler",
        action="store_true",
        help="Include Rekordbox built-in sampler files under the Sampler folders.",
    )
    return parser.parse_args()


def build_query(min_bit_depth: int, key: str, include_sampler: bool) -> str:
    sampler_filter = "" if include_sampler else """
  AND COALESCE(c.FolderPath, '') NOT LIKE '%/Sampler/%'
"""
    return f"""
PRAGMA key = '{key}';
PRAGMA query_only = ON;
.headers on
.mode csv
SELECT
  COALESCE(c.ID, '') AS id,
  COALESCE(c.Title, '') AS title,
  COALESCE(a.Name, c.SrcArtistName, '') AS artist,
  c.FileType AS file_type,
  c.BitDepth AS bit_depth,
  c.SampleRate AS sample_rate,
  c.BitRate AS bitrate,
  c.FolderPath AS full_path
FROM djmdContent c
LEFT JOIN djmdArtist a ON a.ID = c.ArtistID
WHERE
  (
    c.FileType = 5
    OR c.FileType = 4
    OR (c.FileType IN (11, 12) AND COALESCE(c.BitDepth, 0) > {min_bit_depth})
  )
{sampler_filter}ORDER BY
  artist COLLATE NOCASE,
  title COLLATE NOCASE,
  full_path COLLATE NOCASE;
"""


def probe_codec_name(path: Path) -> str | None:
    if not shutil.which("ffprobe"):
        raise FileNotFoundError("ffprobe command not found in PATH (required to detect ALAC files)")

    result = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        text=True,
        capture_output=True,
    )

    if result.returncode != 0:
        return None

    codec_name = result.stdout.strip().lower()
    return codec_name or None


def run_sqlcipher(
    sqlcipher: str, db_path: Path, key: str, min_bit_depth: int, include_sampler: bool
) -> List[Track]:
    if not shutil.which(sqlcipher) and Path(sqlcipher).name == sqlcipher:
        raise FileNotFoundError(f"sqlcipher not found in PATH: {sqlcipher}")
    if not shutil.which("ffprobe"):
        raise FileNotFoundError("ffprobe command not found in PATH (required to detect ALAC files)")
    if not db_path.exists():
        raise FileNotFoundError(f"database file not found: {db_path}")

    sql = build_query(min_bit_depth, key, include_sampler)
    result = subprocess.run(
        [sqlcipher, str(db_path)],
        input=sql,
        text=True,
        capture_output=True,
        env={**os.environ, "LC_ALL": "C"},
    )

    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "sqlcipher failed")

    lines = [line for line in result.stdout.splitlines() if line.strip() and line.strip() != "ok"]
    rows = csv.DictReader(lines)
    tracks: List[Track] = []
    for row in rows:
        file_type = int(row["file_type"]) if row["file_type"] else 0
        full_path = row["full_path"] or ""
        codec_name: str | None = None
        if file_type == 4 and full_path:
            codec_name = probe_codec_name(Path(full_path))
            if codec_name != "alac":
                continue
        tracks.append(
            Track(
                id=row["id"] or "",
                title=row["title"] or "",
                artist=row["artist"] or "",
                file_type=file_type,
                file_type_name="ALAC" if codec_name == "alac" else FILE_TYPE_NAMES.get(file_type, str(file_type)),
                codec_name=codec_name,
                bit_depth=int(row["bit_depth"]) if row["bit_depth"] else None,
                sample_rate=int(row["sample_rate"]) if row["sample_rate"] else None,
                bitrate=int(row["bitrate"]) if row["bitrate"] else None,
                full_path=full_path,
            )
        )
    return tracks


def print_table(tracks: Iterable[Track]) -> None:
    rows = list(tracks)
    headers = ["TITLE", "ARTIST", "TYPE", "CODEC", "BITDEPTH", "SAMPLE_RATE", "BITRATE", "PATH"]
    data = [
        [
            t.title,
            t.artist,
            t.file_type_name,
            "" if t.codec_name is None else t.codec_name,
            "" if t.bit_depth is None else str(t.bit_depth),
            "" if t.sample_rate is None else str(t.sample_rate),
            "" if t.bitrate is None else str(t.bitrate),
            t.full_path,
        ]
        for t in rows
    ]
    widths = [len(h) for h in headers]
    for row in data:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    def fmt_row(row: list[str]) -> str:
        return "  ".join(cell.ljust(widths[i]) for i, cell in enumerate(row))

    print(fmt_row(headers))
    print("  ".join("-" * w for w in widths))
    for row in data:
        print(fmt_row(row))


def print_csv(tracks: Iterable[Track]) -> None:
    writer = csv.DictWriter(
        sys.stdout,
        fieldnames=["id", "title", "artist", "file_type", "codec_name", "bit_depth", "sample_rate", "bitrate", "full_path"],
    )
    writer.writeheader()
    for track in tracks:
        writer.writerow(track.to_dict())


def print_json(tracks: Iterable[Track]) -> None:
    json.dump([track.to_dict() for track in tracks], sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")


def _excel_col_name(index: int) -> str:
    index += 1
    name = ""
    while index:
        index, remainder = divmod(index - 1, 26)
        name = chr(65 + remainder) + name
    return name


def write_xlsx(path: Path, tracks: list[Track]) -> None:
    headers = ["id", "title", "artist", "file_type", "codec_name", "bit_depth", "sample_rate", "bitrate", "full_path"]

    def cell(ref: str, value: object) -> str:
        if value is None:
            return f'<c r="{ref}"/>'
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f'<c r="{ref}"><v>{value}</v></c>'
        text = escape(str(value))
        return f'<c r="{ref}" t="inlineStr"><is><t xml:space="preserve">{text}</t></is></c>'

    rows_xml = []
    all_rows = [headers] + [
        [
            track.id,
            track.title,
            track.artist,
            track.file_type_name,
            track.codec_name,
            track.bit_depth,
            track.sample_rate,
            track.bitrate,
            track.full_path,
        ]
        for track in tracks
    ]
    for row_index, row in enumerate(all_rows, start=1):
        cells = []
        for col_index, value in enumerate(row):
            ref = f"{_excel_col_name(col_index)}{row_index}"
            cells.append(cell(ref, value))
        rows_xml.append(f'<row r="{row_index}">{"".join(cells)}</row>')

    sheet_xml = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    {''.join(rows_xml)}
  </sheetData>
</worksheet>
"""

    workbook_xml = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Tracks" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>
"""

    workbook_rels_xml = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>
"""

    root_rels_xml = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>
"""

    content_types_xml = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>
"""

    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("[Content_Types].xml", content_types_xml)
        zf.writestr("_rels/.rels", root_rels_xml)
        zf.writestr("xl/workbook.xml", workbook_xml)
        zf.writestr("xl/_rels/workbook.xml.rels", workbook_rels_xml)
        zf.writestr("xl/worksheets/sheet1.xml", sheet_xml)


def write_output(tracks: list[Track], output: Path, fmt: str) -> str:
    if fmt == "csv":
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(
                handle,
                fieldnames=["id", "title", "artist", "file_type", "codec_name", "bit_depth", "sample_rate", "bitrate", "full_path"],
            )
            writer.writeheader()
            for track in tracks:
                writer.writerow(track.to_dict())
        return "csv"
    if fmt == "xlsx":
        write_xlsx(output, tracks)
        return "xlsx"
    raise ValueError(f"unsupported file output format: {fmt}")


def main() -> int:
    args = parse_args()
    try:
        tracks = run_sqlcipher(
            args.sqlcipher,
            args.db.expanduser(),
            args.key,
            args.min_bit_depth,
            args.include_sampler,
        )
    except Exception as exc:  # noqa: BLE001
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.output is not None:
        output_format = args.format
        if output_format == "table":
            suffix = args.output.suffix.lower()
            if suffix == ".csv":
                output_format = "csv"
            elif suffix == ".xlsx":
                output_format = "xlsx"
            else:
                print("error: when using --output, specify --format csv|xlsx or use a .csv/.xlsx file name", file=sys.stderr)
                return 1
        try:
            written_format = write_output(tracks, args.output.expanduser(), output_format)
        except Exception as exc:  # noqa: BLE001
            print(f"error: {exc}", file=sys.stderr)
            return 1
        print(f"Wrote {len(tracks)} rows to {args.output.expanduser()} ({written_format})")
        return 0

    if args.format == "table":
        print(f"Database: {args.db.expanduser()}")
        print(f"Matches: {len(tracks)}")
        print_table(tracks)
    elif args.format == "csv":
        print_csv(tracks)
    elif args.format == "xlsx":
        print("error: --format xlsx requires --output", file=sys.stderr)
        return 1
    else:
        print_json(tracks)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
