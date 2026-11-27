# unused-ok: time-just-now time-minutes-ago time-hours-ago time-days-ago
# (the relative-time ids are chosen at run time in src/format.rs)
time-just-now = just now
time-minutes-ago =
    { $count ->
        [one] { $count } minute ago
       *[other] { $count } minutes ago
    }
time-hours-ago =
    { $count ->
        [one] { $count } hour ago
       *[other] { $count } hours ago
    }
time-days-ago =
    { $count ->
        [one] { $count } day ago
       *[other] { $count } days ago
    }
