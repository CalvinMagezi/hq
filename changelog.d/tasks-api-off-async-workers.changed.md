**The task screens stay responsive under load.** The task API now runs its database work on a blocking
thread instead of an async worker, so a slow query no longer stalls chat and other requests.
