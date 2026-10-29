from __future__ import annotations

import faulthandler
import os

from .runner import main as run


def main() -> None:
    timeout = os.environ.get("MOLI_SMOKE_WORKER_TIMEOUT_SECONDS")
    if timeout is not None:
        # This watchdog uses its own thread: it still records Python stacks if
        # the event loop deadlocks and cannot publish another checkpoint.
        faulthandler.dump_traceback_later(float(timeout) * 0.9)
    try:
        run()
    finally:
        if timeout is not None:
            faulthandler.cancel_dump_traceback_later()


if __name__ == "__main__":
    main()
