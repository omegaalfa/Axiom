<?php
// Runtime declarations exported from extension: sysvshm

namespace {
    function shm_attach(int $key, ?int $size = NULL, int $permissions = 438): \SysvSharedMemory|false {}

    function shm_detach(\SysvSharedMemory $shm): true {}

    function shm_has_var(\SysvSharedMemory $shm, int $key): bool {}

    function shm_remove(\SysvSharedMemory $shm): bool {}

    function shm_put_var(\SysvSharedMemory $shm, int $key, mixed $value): bool {}

    function shm_get_var(\SysvSharedMemory $shm, int $key): mixed {}

    function shm_remove_var(\SysvSharedMemory $shm, int $key): bool {}

    final class SysvSharedMemory {
    }

}
