#!/usr/bin/env python3
"""Byte-level helpers for the media fixtures (see README.md). Standard library only.

FFmpeg's command line cannot write some metadata that phone files carry (EXIF orientation,
Dolby Vision configuration, HDR mastering boxes, HEIF tile grids), so these are added here to
files FFmpeg made.
"""

import random
import struct
import sys

# ---------------------------------------------------------------------------------------------
# ISO BMFF (MP4/MOV/HEIF) boxes


def box(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I", 8 + len(payload)) + fourcc + payload


def full_box(fourcc: bytes, version: int, flags: int, payload: bytes) -> bytes:
    return box(fourcc, struct.pack(">I", (version << 24) | flags) + payload)


def children(data: bytes, start: int, end: int):
    """Yields (offset, size, fourcc) of the boxes in data[start:end]."""
    pos = start
    while pos + 8 <= end:
        size, fourcc = struct.unpack(">I4s", data[pos : pos + 8])
        if size < 8:
            raise ValueError(f"unsupported box size {size} at {pos}")
        yield pos, size, fourcc
        pos += size


# Header bytes before the child boxes, per container box.
CONTAINER_HEADER = {b"moov": 8, b"trak": 8, b"mdia": 8, b"minf": 8, b"stbl": 8, b"stsd": 16}
# A visual sample entry has 78 bytes of fields after its 8-byte header.
VISUAL_SAMPLE_ENTRY_HEADER = 8 + 78


def insert_into_sample_entry(data: bytes, entry_fourcc: bytes, extra: bytes) -> bytes:
    """Appends `extra` (whole boxes) to the first sample entry `entry_fourcc` and fixes the sizes of
    every ancestor. Only for files whose moov follows the mdat (no faststart), so no chunk offset
    moves."""
    path = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"]
    ancestors = []  # offsets of the boxes whose size grows

    def find(start, end, depth):
        for off, size, fourcc in children(data, start, end):
            if depth == len(path):
                if fourcc == entry_fourcc:
                    return off, size
                continue
            if fourcc == path[depth]:
                if fourcc == b"moov" and off < mdat_offset:
                    raise ValueError("moov before mdat: chunk offsets would move")
                found = find(off + CONTAINER_HEADER[fourcc], off + size, depth + 1)
                if found:
                    ancestors.append(off)
                    return found
        return None

    mdat_offset = next(off for off, _, fourcc in children(data, 0, len(data)) if fourcc == b"mdat")
    found = find(0, len(data), 0)
    if not found:
        raise ValueError(f"no {entry_fourcc!r} sample entry")
    entry_off, entry_size = found
    ancestors.append(entry_off)
    out = bytearray(data)
    insert_at = entry_off + entry_size
    out[insert_at:insert_at] = extra
    for off in ancestors:
        (size,) = struct.unpack(">I", out[off : off + 4])
        out[off : off + 4] = struct.pack(">I", size + len(extra))
    return bytes(out)


def dolby_vision_box(profile: int, level: int, compatibility_id: int) -> bytes:
    """`dvvC` (profile > 7): Dolby Vision configuration with an RPU and a base layer, no
    enhancement layer."""
    bits = (profile << 9) | (level << 3) | (1 << 2) | (0 << 1) | 1
    payload = bytes([1, 0]) + struct.pack(">H", bits) + bytes([compatibility_id << 4]) + bytes(19)
    return box(b"dvvC", payload)


def mastering_box(max_nits: int, min_nits_x10000: int) -> bytes:
    """`mdcv`: BT.2020 primaries, D65 white, luminance in 0.0001 cd/m2."""
    primaries = [(8500, 39850), (6550, 2300), (35400, 14600)]  # G, B, R in 0.00002 units
    payload = b"".join(struct.pack(">HH", x, y) for x, y in primaries)
    payload += struct.pack(">HH", 15635, 16450)
    payload += struct.pack(">II", max_nits * 10000, min_nits_x10000)
    return box(b"mdcv", payload)


def light_level_box(max_cll: int, max_fall: int) -> bytes:
    return box(b"clli", struct.pack(">HH", max_cll, max_fall))


# ---------------------------------------------------------------------------------------------
# JPEG EXIF


def exif_orientation(jpeg: bytes, orientation: int) -> bytes:
    """Inserts an APP1 Exif segment with one IFD0 entry, Orientation (0x0112), after SOI."""
    if jpeg[:2] != b"\xff\xd8":
        raise ValueError("not a JPEG")
    tiff = b"MM\x00\x2a" + struct.pack(">I", 8)
    tiff += struct.pack(">H", 1)  # one entry
    tiff += struct.pack(">HHIHH", 0x0112, 3, 1, orientation, 0)
    tiff += struct.pack(">I", 0)  # no next IFD
    segment = b"Exif\x00\x00" + tiff
    app1 = b"\xff\xe1" + struct.pack(">H", len(segment) + 2) + segment
    return jpeg[:2] + app1 + jpeg[2:]


# ---------------------------------------------------------------------------------------------
# HEIF (HEIC) still image with a tile grid, like phone photos


def hevc_nal_units(annexb: bytes):
    """Splits an Annex B HEVC stream into NAL units (without start codes)."""
    units, i, start = [], 0, None
    while i + 3 <= len(annexb):
        if annexb[i : i + 3] == b"\x00\x00\x01":
            if start is not None:
                units.append(annexb[start:i].rstrip(b"\x00"))
            i += 3
            start = i
        else:
            i += 1
    if start is not None:
        units.append(annexb[start:])
    return [u for u in units if u]


def unescape(unit: bytes) -> bytes:
    """Removes emulation prevention bytes (00 00 03 -> 00 00)."""
    out, zeros = bytearray(), 0
    for b in unit:
        if zeros >= 2 and b == 3:
            zeros = 0
            continue
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def nal_type(unit: bytes) -> int:
    return (unit[0] >> 1) & 0x3F


def hvcc(vps: bytes, sps: bytes, pps: bytes) -> bytes:
    """A minimal HEVCDecoderConfigurationRecord. FFmpeg's decoder reads the parameter sets from the
    arrays; the profile fields are copied from the SPS profile_tier_level."""
    raw = unescape(sps)
    ptl = raw[2 + 1 : 2 + 1 + 12]  # after the 2-byte NAL header and 1 byte of sps ids
    general = ptl[0]
    compat = ptl[1:5]
    constraints = ptl[5:11]
    level = ptl[11]
    record = bytes([1, general]) + compat + constraints + bytes([level])
    record += struct.pack(">H", 0xF000)  # min_spatial_segmentation_idc = 0
    record += bytes([0xFC])  # parallelismType = 0
    record += bytes([0xFD])  # chroma_format_idc = 1 (4:2:0)
    record += bytes([0xF8, 0xF8])  # 8-bit luma and chroma
    record += struct.pack(">H", 0)  # avgFrameRate
    record += bytes([0x0F])  # constantFrameRate 0, numTemporalLayers 1, nested 1, lengthSize 4
    arrays = [(32, vps), (33, sps), (34, pps)]
    record += bytes([len(arrays)])
    for kind, unit in arrays:
        record += bytes([0x80 | kind]) + struct.pack(">HH", 1, len(unit)) + unit
    return record


def heif_grid(tiles_annexb, tile_w: int, tile_h: int, cols: int, rows: int, rotation_ccw: int) -> bytes:
    """A HEIF file whose primary item is a `grid` of `cols` x `rows` HEVC tiles, each tile one
    Annex B stream of a single intra picture, with an `irot` property on the grid."""
    tiles = []
    config = None
    for annexb in tiles_annexb:
        units = hevc_nal_units(annexb)
        params = {t: u for u in units if (t := nal_type(u)) in (32, 33, 34)}
        if config is None:
            config = hvcc(params[32], params[33], params[34])
        slices = [u for u in units if nal_type(u) < 32]
        tiles.append(b"".join(struct.pack(">I", len(u)) + u for u in slices))

    grid_id = 1
    tile_ids = list(range(2, 2 + len(tiles)))
    out_w, out_h = cols * tile_w, rows * tile_h
    grid_data = bytes([0, 0, rows - 1, cols - 1]) + struct.pack(">HH", out_w, out_h)

    ftyp = box(b"ftyp", b"heic" + struct.pack(">I", 0) + b"mif1heic")
    hdlr = full_box(b"hdlr", 0, 0, bytes(4) + b"pict" + bytes(12) + b"\x00")
    pitm = full_box(b"pitm", 0, 0, struct.pack(">H", grid_id))
    infe = [full_box(b"infe", 2, 0, struct.pack(">HH", grid_id, 0) + b"grid" + b"\x00")]
    infe += [full_box(b"infe", 2, 1, struct.pack(">HH", i, 0) + b"hvc1" + b"\x00") for i in tile_ids]
    iinf = full_box(b"iinf", 0, 0, struct.pack(">H", len(infe)) + b"".join(infe))
    dimg = struct.pack(">HH", grid_id, len(tile_ids)) + b"".join(struct.pack(">H", i) for i in tile_ids)
    iref = full_box(b"iref", 0, 0, box(b"dimg", dimg))

    # Properties: 1 hvcC, 2 ispe(tile), 3 ispe(grid), 4 irot.
    ipco = box(
        b"ipco",
        box(b"hvcC", config)
        + full_box(b"ispe", 0, 0, struct.pack(">II", tile_w, tile_h))
        + full_box(b"ispe", 0, 0, struct.pack(">II", out_w, out_h))
        + box(b"irot", bytes([(rotation_ccw // 90) & 3])),
    )
    assoc = struct.pack(">H", grid_id) + bytes([2, 3, 0x80 | 4])
    for i in tile_ids:
        assoc += struct.pack(">H", i) + bytes([2, 0x80 | 1, 2])
    ipma = full_box(b"ipma", 0, 0, struct.pack(">I", 1 + len(tile_ids)) + assoc)
    iprp = box(b"iprp", ipco + ipma)

    payloads = [grid_data] + tiles
    ids = [grid_id] + tile_ids

    def build(base: int) -> bytes:
        entries = b""
        offset = base
        for item_id, payload in zip(ids, payloads):
            # version 0, offset_size 4, length_size 4, base_offset_size 0
            entries += struct.pack(">HHHII", item_id, 0, 1, offset, len(payload))
            offset += len(payload)
        iloc = full_box(b"iloc", 0, 0, bytes([0x44, 0x00]) + struct.pack(">H", len(ids)) + entries)
        # grid data is stored in mdat like the tiles (construction method 0)
        meta = full_box(b"meta", 0, 0, hdlr + pitm + iloc + iinf + iref + iprp)
        return meta

    head_len = len(ftyp) + len(build(0)) + 8
    meta = build(head_len)
    mdat = box(b"mdat", b"".join(payloads))
    return ftyp + meta + mdat


# ---------------------------------------------------------------------------------------------


def main(argv):
    cmd = argv[1]
    if cmd == "dolby-vision":
        src, dst, entry, profile, level, compat = argv[2:8]
        data = open(src, "rb").read()
        extra = dolby_vision_box(int(profile), int(level), int(compat))
        open(dst, "wb").write(insert_into_sample_entry(data, entry.encode(), extra))
    elif cmd == "hdr10-boxes":
        src, dst, entry, max_nits, max_cll, max_fall = argv[2:8]
        data = open(src, "rb").read()
        extra = mastering_box(int(max_nits), 50) + light_level_box(int(max_cll), int(max_fall))
        open(dst, "wb").write(insert_into_sample_entry(data, entry.encode(), extra))
    elif cmd == "exif-orientation":
        src, dst, orientation = argv[2:5]
        open(dst, "wb").write(exif_orientation(open(src, "rb").read(), int(orientation)))
    elif cmd == "heif-grid":
        dst, tile_w, tile_h, cols, rows, rotation = argv[2:8]
        tiles = [open(p, "rb").read() for p in argv[8:]]
        data = heif_grid(tiles, int(tile_w), int(tile_h), int(cols), int(rows), int(rotation))
        open(dst, "wb").write(data)
    elif cmd == "random":
        dst, length, seed = argv[2:5]
        rng = random.Random(int(seed))
        open(dst, "wb").write(bytes(rng.getrandbits(8) for _ in range(int(length))))
    elif cmd == "truncate":
        src, dst, keep_percent = argv[2:5]
        data = open(src, "rb").read()
        open(dst, "wb").write(data[: len(data) * int(keep_percent) // 100])
    else:
        raise SystemExit(f"unknown command {cmd}")


if __name__ == "__main__":
    main(sys.argv)
