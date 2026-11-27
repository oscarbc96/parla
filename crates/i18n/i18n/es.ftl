time-just-now = ahora mismo
time-minutes-ago =
    { $count ->
        [one] hace { $count } minuto
       *[other] hace { $count } minutos
    }
time-hours-ago =
    { $count ->
        [one] hace { $count } hora
       *[other] hace { $count } horas
    }
time-days-ago =
    { $count ->
        [one] hace { $count } día
       *[other] hace { $count } días
    }
