<?php

namespace App\Service;

use App\Repository\UserRepository;

final class UserService
{
    public function __construct(
        private UserRepository $repository,
    ) {}

    public function run(string $email): void
    {
        $this->repository->findByEmail($email);
    }
    
    public function findByEmail(string $email): string
    {
        return $email;
    }
}
