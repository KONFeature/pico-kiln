/* RP2350A (Raspberry Pi Pico 2 W) memory layout.
 *
 * 4 MiB external QSPI flash mapped at the XIP base, 520 KiB on-chip SRAM. The
 * RP2350 bootrom requires an IMAGE_DEF block near the start of flash; embassy-rp
 * emits one into `.start_block` (default `secure_exe`), and a matching
 * `.end_block` at the tail. The INSERT directives below splice those sections
 * into cortex-m-rt's `link.x` so the ROM can find and validate the image. */

/* The linker may fill only the bottom 2560 KiB of the 4 MiB flash; the top
 * 1536 KiB (offset 0x280000..0x400000) is reserved for the littlefs2 partition
 * (config + profiles + logs — see platform.rs FS_BASE/FS_SIZE). Capping FLASH
 * here guarantees the image can never grow into the filesystem region. */
MEMORY {
    FLASH : ORIGIN = 0x10000000, LENGTH = 2560K
    RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}

SECTIONS {
    /* The boot IMAGE_DEF block, immediately after the vector table. */
    .start_block : ALIGN(4)
    {
        __start_block_addr = .;
        KEEP(*(.start_block));
        KEEP(*(.boot_info));
    } > FLASH
} INSERT AFTER .vector_table;

/* Move the entry point past the start block. */
_stext = ADDR(.start_block) + SIZEOF(.start_block);

SECTIONS {
    /* Picotool "Binary Info" entries (optional metadata). */
    .bi_entries : ALIGN(4)
    {
        __bi_entries_start = .;
        KEEP(*(.bi_entries));
        . = ALIGN(4);
        __bi_entries_end = .;
    } > FLASH
} INSERT AFTER .text;

SECTIONS {
    /* The closing block the bootrom uses to delimit the image. */
    .end_block : ALIGN(4)
    {
        __end_block_addr = .;
        KEEP(*(.end_block));
    } > FLASH
} INSERT AFTER .bi_entries;

PROVIDE(start_to_end = __end_block_addr - __start_block_addr);
PROVIDE(end_to_start = __start_block_addr - __end_block_addr);
