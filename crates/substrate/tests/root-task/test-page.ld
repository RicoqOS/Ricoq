/* The image-layout test verifies that this ELF header is the first loaded image page. */
__root_image_start = __ehdr_start;

SECTIONS {
    .task_code ALIGN(4096) : {
        __task_code_start = .;
        KEEP(*(.task_code))
        . = __task_code_start + 4096;
        __task_code_end = .;
    }

    .task_scratch ALIGN(4096) : {
        __task_scratch_start = .;
        KEEP(*(.task_scratch))
        __task_scratch_end = .;
    }
}
INSERT AFTER .data;

ASSERT(SIZEOF(.task_code) == 4096, "task code must occupy one page")
ASSERT((ADDR(.task_code) % 4096) == 0, "task code must be page aligned")
ASSERT(SIZEOF(.task_scratch) == 4096, "scratch reservation must occupy one page")
ASSERT((ADDR(.task_scratch) % 4096) == 0, "scratch reservation must be page aligned")
