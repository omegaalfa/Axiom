<?php

declare(strict_types=1);

namespace Probe\BaseA {

    class Service
    {
    }
}

namespace Probe\BaseB {

    class Service
    {
    }
}

namespace Probe\ModelsA {

    use Probe\BaseA\Service;

    class UserService extends Service
    {
    }

    class AdminService extends Service
    {
    }

    class SpecialUserService extends UserService
    {
    }
}

namespace Probe\ModelsB {

    use Probe\BaseB\Service;

    class OrderService extends Service
    {
    }
}

namespace Probe\AliasModels {

    use Probe\BaseA\Service as BaseService;

    class AliasedService extends BaseService
    {
    }
}

namespace Probe\Usage {

    use Probe\BaseA\Service as ServiceA;
    use Probe\BaseB\Service as ServiceB;
    use Probe\ModelsA\UserService;

    function exercise(
        ServiceA $serviceA,
        ServiceB $serviceB,
        UserService $userService,
    ): void {
    }
}