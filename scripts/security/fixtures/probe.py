"""A canary-leak probe confined to the fixed internal Docker target alias."""
import json
import sys
import time
import urllib.parse
import urllib.request


# No caller-selected host is accepted by the probe.
def request(name):
    url = "http://target:8080/?" + urllib.parse.urlencode({"name": name})
    with urllib.request.urlopen(url, timeout=2) as response:
        return json.load(response)


# Readiness failures are infrastructure, never evidence that a patch blocks the probe.
def main():
    mode = sys.argv[1]
    if mode == "ready":
        for _ in range(20):
            try:
                if request("public") == [["hello"]]:
                    return 0
            except (OSError, ValueError):
                time.sleep(0.25)
        return 2
    if mode != "probe":
        return 2
    try:
        if request("missing") != []:
            return 2
        result = request("' OR 1=1 -- ")
        return 0 if ["lab-canary"] in result else 1
    except (OSError, ValueError):
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
