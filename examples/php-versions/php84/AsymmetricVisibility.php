<?php

namespace PhpPlayground\Php84;

class UserProfile
{
    public private(set) string $name;
    protected private(set) ?int $id = null;
    public readonly private(set) string $uuid;

    public function __construct(string $name, string $uuid, ?int $id = null)
    {
        $this->name = $name;
        $this->uuid = $uuid;
        $this->id = $id;
    }

    public function updateProfile(string $newName, int $newId): void
    {
        $this->name = $newName;
        $this->id = $newId;
    }

    public function getProfileInfo(): array
    {
        return [
            'name' => $this->name,
            'id' => $this->id,
            'uuid' => $this->uuid,
        ];
    }
}
