"""Check the test-page/BootInfo frame-index contract in the actual linked ELF."""

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
    assert ".vspace_test" in by_name, "missing dedicated test-page section"
    page = by_name[".vspace_test"]
    assert page[3] % 4096 == 0 and page[5] == 4096, "test page must be one granule"
    assert page[2] & 3 == 3 and page[1] == 1, "test page must be writable PROGBITS"
    assert section_data(page) == b"\xee" * 4096, "wrong test-page storage"

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
    assert symbols["__vspace_test_start"] == page[3], "wrong test-page start"
    assert symbols["__vspace_test_end"] == page[3] + 4096, "wrong test-page end"
    assert any(
        segment[1] & 6 == 6
        and segment[3] <= page[3]
        and page[3] + 4096 <= segment[3] + segment[5]
        and page[4] - segment[2] == page[3] - segment[3]
        for segment in loads
    ), "test page must be file-backed in a readable/writable PT_LOAD"
    for section in sections:
        if section is page or not section[2] & 2 or section[5] == 0:
            continue
        assert (
            section[3] + section[5] <= page[3]
            or section[3] >= page[3] + 4096
        ), "live section overlaps the test page"
    print("IMAGE_LAYOUT: PASS")


def check_production(path):
    data = Path(path).read_bytes()
    for marker in (b"TEST_RESULT:", b".vspace_test", b"vspace: frame allocated"):
        assert marker not in data, "production image contains the mapping exercise"
    print("PRODUCTION_IMAGE: PASS")


if __name__ == "__main__":
    if sys.argv[1] == "--production":
        check_production(sys.argv[2])
    else:
        check(sys.argv[1])
