"""Check task-code, scratch-page, and BootInfo contracts in the linked ELF."""

import struct
import sys
from pathlib import Path


def check(path):
    data = Path(path).read_bytes()
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    assert header[0][:6] == b"\x7fELF\x02\x01", "expected ELF64 little endian"
    assert header[2] == 183, "expected AArch64"
    segments = [
        struct.unpack_from("<IIQQQQQQ", data, header[5] + i * header[9])
        for i in range(header[10])
    ]
    sections = [
        struct.unpack_from("<IIQQQQIIQQ", data, header[6] + i * header[11])
        for i in range(header[12])
    ]

    def section_data(section):
        return data[section[4]:section[4] + section[5]]

    def name(strings, offset):
        return strings[offset:strings.index(b"\0", offset)].decode()

    names = section_data(sections[header[13]])
    by_name = {name(names, section[0]): section for section in sections}
    assert ".task_code" in by_name, "missing dedicated task-code section"
    assert ".task_scratch" in by_name, "missing dedicated scratch section"
    code = by_name[".task_code"]
    scratch = by_name[".task_scratch"]
    for section in (code, scratch):
        assert section[3] % 4096 == 0, "task section must be page aligned"
        assert section[5] == 4096, "task section must occupy one granule"
        assert section[1] == 1, "task section must be file-backed PROGBITS"
    assert code[2] & 6 == 6 and not code[2] & 1, "task code must be read/execute"
    assert scratch[2] & 3 == 3 and not scratch[2] & 4, "scratch must be read/write"
    assert section_data(code) != b"\0" * 4096, "task code must not be empty"
    assert section_data(scratch) == b"\xee" * 4096, "wrong scratch storage"

    symbols = {}
    for section in sections:
        if section[1] != 2:
            continue
        strings = section_data(sections[section[6]])
        for offset in range(section[4], section[4] + section[5], section[9]):
            symbol = struct.unpack_from("<IBBHQQ", data, offset)
            symbols[name(strings, symbol[0])] = symbol[4]

    loads = [segment for segment in segments if segment[0] == 1]
    image_start = min(segment[3] for segment in loads)
    assert image_start % 4096 == 0, "image base must be page aligned"
    assert symbols["__root_image_start"] == image_start, "wrong BootInfo frame zero"
    for section, prefix, flags in (
        (code, "__task_code", 5),
        (scratch, "__task_scratch", 6),
    ):
        assert symbols[f"{prefix}_start"] == section[3], f"wrong {prefix} start"
        assert symbols[f"{prefix}_end"] == section[3] + 4096, f"wrong {prefix} end"
        assert any(
            segment[1] & flags == flags
            and segment[3] <= section[3]
            and section[3] + 4096 <= segment[3] + segment[5]
            and section[4] - segment[2] == section[3] - segment[3]
            for segment in loads
        ), f"{prefix} must be file-backed in a matching PT_LOAD"
        for other in sections:
            if other is section or not other[2] & 2 or other[5] == 0:
                continue
            assert (
                other[3] + other[5] <= section[3]
                or other[3] >= section[3] + 4096
            ), f"live section overlaps {prefix}"
    print("IMAGE_LAYOUT: PASS")


def check_production(path):
    data = Path(path).read_bytes()
    for marker in (b"TEST_RESULT:", b".task_code", b"task: resources constructed"):
        assert marker not in data, "production image contains the mapping exercise"
    print("PRODUCTION_IMAGE: PASS")


if __name__ == "__main__":
    if sys.argv[1] == "--production":
        check_production(sys.argv[2])
    else:
        check(sys.argv[1])
