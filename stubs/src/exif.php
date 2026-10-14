<?php
// Runtime declarations exported from extension: exif

namespace {
    function exif_tagname(int $index): string|false {}

    function exif_read_data($file, ?string $required_sections = NULL, bool $as_arrays = false, bool $read_thumbnail = false): array|false {}

    function exif_thumbnail($file, &$width = NULL, &$height = NULL, &$image_type = NULL): string|false {}

    function exif_imagetype(string $filename): int|false {}

    const EXIF_USE_MBSTRING = 1;

}
