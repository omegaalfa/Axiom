<?php
// Runtime declarations exported from extension: sysvmsg

namespace {
    function msg_get_queue(int $key, int $permissions = 438): \SysvMessageQueue|false {}

    function msg_send(\SysvMessageQueue $queue, int $message_type, $message, bool $serialize = true, bool $blocking = true, &$error_code = NULL): bool {}

    function msg_receive(\SysvMessageQueue $queue, int $desired_message_type, &$received_message_type, int $max_message_size, mixed &$message, bool $unserialize = true, int $flags = 0, &$error_code = NULL): bool {}

    function msg_remove_queue(\SysvMessageQueue $queue): bool {}

    function msg_stat_queue(\SysvMessageQueue $queue): array|false {}

    function msg_set_queue(\SysvMessageQueue $queue, array $data): bool {}

    function msg_queue_exists(int $key): bool {}

    final class SysvMessageQueue {
    }

    const MSG_IPC_NOWAIT = 1;

    const MSG_EAGAIN = 11;

    const MSG_ENOMSG = 42;

    const MSG_NOERROR = 2;

    const MSG_EXCEPT = 4;

}
