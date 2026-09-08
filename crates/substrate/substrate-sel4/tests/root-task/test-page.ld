/* The image-layout test verifies that this ELF header is the first loaded image page. */
__root_image_start = __ehdr_start;

SECTIONS {
    .vspace_test ALIGN(4096) : {
        __vspace_test_start = .;
        KEEP(*(.vspace_test))
        __vspace_test_end = .;
    }
}
INSERT AFTER .data;

ASSERT(SIZEOF(.vspace_test) == 4096, "test reservation must occupy one page")
ASSERT((ADDR(.vspace_test) % 4096) == 0, "test reservation must be page aligned")
