# CDP response body retention

`Network.configureDurableMessages` configures response body retention for the
calling session without enabling or disabling Network events, and without
waiting for renderer policy replay. Call `Network.enable` to receive events and
use `Network.getResponseBody`.

```json
{"method":"Network.enable"}
{"method":"Network.configureDurableMessages","params":{"maxTotalBufferSize":10000000,"maxResourceBufferSize":5000000}}
```

Completed response bodies captured while retention is configured can survive
subsequent navigations on the same target. Each session owns its own FIFO quota
and retention claims. A smaller budget or new traffic in another session cannot
evict those claims. The payload allocation or spool is shared through reference
counting; it is released after the current document cache and all collectors
release it. Root commands and the target's explicit primary session ID address
the same collector. Other attached sessions and targets remain separate.

The total and per-resource limits are measured in bytes. An omitted per-resource
limit uses the configured total. Explicit durable budgets are not clamped to the
ordinary document cache's 20,000,000-byte total / 2,000,000-byte resource defaults.
Reconfiguring an existing collector grows either limit but does not shrink it;
disable and configure again to start with a smaller budget. Disabling releases
that collector's retained bodies immediately. It does not clear ordinary bodies
from the current document or turn off Network events.

`Network.configureDurableMessages` with no `maxTotalBufferSize`, or a total of
zero, disables retention. A per-resource size of zero accepts only empty bodies.
Negative sizes or non-integer sizes are invalid. The legacy
`Network.enable(enableDurableMessages=true)` entry point requires a positive
total; explicitly passing `enableDurableMessages=false` disables retention.
Omitting that flag leaves an independently configured collector intact.

Moli deliberately limits each durable collector to 4096 records, including
empty bodies and failure/eviction metadata, to bound metadata that is not charged
to the payload byte quota. The oldest records are removed when this limit is
reached. This is a Moli resource protection policy, not a CDP parameter.
Disabling Network or detaching a session releases its claims; closing the target
releases all its collectors. Pending responses and BiDi collection rights do not
become durable through these CDP commands. Response materialization remains
subject to Moli's separate read-size limit.

Compatibility references:

- [CDP Network commands](https://chromedevtools.github.io/devtools-protocol/tot/Network/#method-configureDurableMessages)
- [Chromium Network handler](https://github.com/chromium/chromium/blob/main/content/browser/devtools/protocol/network_handler.cc)
- [Chromium durable collector](https://github.com/chromium/chromium/blob/main/services/network/devtools_durable_msg_collector.cc)
