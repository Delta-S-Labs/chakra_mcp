#!/usr/bin/env python3
"""The end-to-end test's web browser.

As $BROWSER, `chakramcp login --method browser` runs `browser.py <url>` to
open the server's /oauth/authorize page. This signs in and approves there,
so the CLI's loopback listener gets its code. It forks and returns at once
(the CLI waits for its browser to start, then for the callback) and logs to
$E2E_LOG_DIR/browser.log, because the CLI discards a browser's output.

`browser.py pair <url>` approves a device pairing the same way (run.sh
calls it directly).
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pages  # noqa: E402


def main():
    args = sys.argv[1:]
    mode, url = ("pair", args[1]) if args[:1] == ["pair"] else ("authorize", args[0])
    email = os.environ["E2E_ADMIN_EMAIL"]
    password = os.environ["E2E_ADMIN_PASSWORD"]
    log_path = os.path.join(os.environ.get("E2E_LOG_DIR", "."), f"browser-{mode}.log")

    if mode == "authorize" and os.fork() != 0:
        return 0  # the CLI's spawn returns; the child does the work

    with open(log_path, "a") as log:
        def say(line):
            print(line, file=log, flush=True)

        browser = pages.Browser(log=say)
        try:
            if mode == "pair":
                pages.pair(
                    browser, url, email, password,
                    os.environ.get("E2E_AGENT_NAME", "E2E Agent"),
                    os.environ.get("E2E_AGENT_SLUG", "e2e-agent"),
                )
            else:
                final = pages.authorize(browser, url, email, password)
                say(f"callback reached: {final.url.split('?')[0]} {final.status}")
        except Exception as err:  # the log is the only place this shows
            say(f"FAILED: {err}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
