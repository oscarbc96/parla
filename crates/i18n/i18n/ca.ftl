time-just-now = ara mateix
time-minutes-ago =
    { $count ->
        [one] fa { $count } minut
       *[other] fa { $count } minuts
    }
time-hours-ago =
    { $count ->
        [one] fa { $count } hora
       *[other] fa { $count } hores
    }
time-days-ago =
    { $count ->
        [one] fa { $count } dia
       *[other] fa { $count } dies
    }
