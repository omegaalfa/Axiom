<?php

namespace PhpPlayground\Php80;

class MatchExample
{
    public string|int|null $id = null;

    public function evaluateStatus(string|int $status): string
    {
        return match ($status) {
            'active', 1 => 'Active status',
            'pending', 2 => 'Pending status',
            'suspended', 'banned', 3 => 'Inactive status',
            default => 'Unknown status',
        };
    }
}
