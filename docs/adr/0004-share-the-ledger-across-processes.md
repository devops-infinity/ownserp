# 0004 Share the ledger across processes

Date: 2026-10-25

## Context

Several ownserp processes can run against one ledger file: a server for one client, a second client on the same machine, or a batch run started by the operator. Each one appends rows, reads the totals, and holds a budget reservation. The chain hash means a row is valid only when it follows the row before it, so two writers must never chain onto the same tail.

A lock held for the life of the process is too coarse. It blocks every other process from reading or writing, and one stalled process stalls all of them. The design has to keep one chain, let several processes make progress, and keep cost reporting off the file lock.

## Decision

Each operation takes the file lock for its own duration and releases it at the end. Reading the tail, checking the chain, and appending one row (or a batch of queued rows) happen inside the exclusive lock, and the file is synced before the lock drops. A read of the totals takes a shared lock. No process holds the lock between operations.

Cost reporting reads a published snapshot, not the file. Each successful append and each totals read publishes the totals it computed into a snapshot behind its own mutex, and the cost object reads that snapshot. The snapshot has its own lock so a cost report never waits on the ledger file lock held by another process, and never waits on the state mutex while a write is in flight.

Every wait for the file lock is bounded at 5 seconds. A process polls the lock every 10 ms and gives up after 5 seconds instead of blocking forever.

A lock that runs out of time is reported as a busy ledger, not as damage. The busy error carries its own code and remedy, leaves the ledger healthy, and lets the next call try again. Only an error that could have damaged the file marks the ledger broken and refuses later budgeted calls until the process restarts.

A paid call that cannot be written because the lock is busy is not lost silently. The ledger holds a bounded queue of 256 unwritten call records behind its own lock, separate from the state mutex, and pushes the record when an append meets a busy lock. The next successful append writes the queued records first, in order, then its own record, in one exclusive lock hold with one sync at the end. Each record keeps the time the call was observed, so its row carries the real time. When the queue is full, the oldest record is dropped and one error log records how many were lost. Queued credits count against the budget alongside written credits and reservations already held. The queue is flushed best effort when the last ledger handle drops, using the normal bounded wait, and a queued record is not visible in a cost report until it is written.

A failure is recorded at zero credits when Serper did not bill the call: a 5xx answer, a 3xx redirect, a 4xx rejection, and a rate limit. A timeout, a cancel, or an error after a good response keeps the estimate, because the call may have been billed. This follows ADR 0002, which says Serper bills only successful calls.

The session budget is the credits written to the current ledger chain, counted across process restarts, and a rotate starts a fresh count. It is not a per-process figure and a restart does not reset it. A budget reservation is held in process memory and released when the call settles.

## Consequences

Two processes cannot fork the chain, and neither can chain onto a partial row, because every append re-reads the tail and checks the chain under the exclusive lock. A process that stalls inside the lock delays the others by at most 5 seconds per operation, and a call that meets a busy lock still returns its result, with its row written by the next append in the same process.

The bounded queue trades a small memory cost for no lost paid calls in the common stall case. It is not a durable outbox: a crash with queued records loses them, and the drop rule loses the oldest when the queue fills. The queue is per process, so two processes queue separately.

Cross-process budget overshoot remains possible, because a reservation is released before the call runs and each process sees only the written totals plus its own held credits. The overshoot is bounded by the number of running processes times the largest single reservation, which is the per-call credit ceiling when one is set.
