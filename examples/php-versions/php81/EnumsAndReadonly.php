<?php

namespace PhpPlayground\Php81;

enum Status: string
{
    case Pending = 'pending';
    case Active = 'active';
}

class UserDTO
{
    public readonly string $email;

    public function __construct(string $email)
    {
        $this->email = $email;
    }
}
