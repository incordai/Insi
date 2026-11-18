# Copyright (c) 2026-2027 Incord Ai.
# https://incord.ai
#
# SPDX-License-Identifier: BUSL-1.1

"""Import Nmap XML as attack-surface observations; never run a network scan."""

import argparse
import json
import ipaddress
from pathlib import Path, PurePosixPath
import xml.etree.ElementTree as ET


# Bind observations to the trusted repository configuration that defines a lab service.
def convert(raw, file, line):
    path = PurePosixPath(file)
    if not file or path.is_absolute() or ".." in path.parts or "\\" in file or ":" in file or line < 1:
        raise ValueError("anchor must be relative with a positive line")
    if len(raw) > 1024 * 1024 or b"<!ENTITY" in raw.upper():
        raise ValueError("Nmap report too large or contains entity definitions")
    root = ET.fromstring(raw)
    finished = root.find("runstats/finished")
    if root.tag != "nmaprun" or finished is None or finished.get("exit") != "success":
        raise ValueError("Nmap scan is incomplete")
    findings = []
    for host in root.findall("host"):
        address = host.find("address")
        if address is None:
            raise ValueError("Nmap host address missing")
        # Addresses are data, never arbitrary scanner strings in a published message.
        host_address = str(ipaddress.ip_address(address.get("addr", "")))
        for port in host.findall("ports/port"):
            state = port.find("state")
            if state is None or state.get("state") != "open":
                continue
            number = int(port.get("portid", "0"))
            protocol = port.get("protocol")
            if not 1 <= number <= 65535 or protocol not in ("tcp", "udp", "sctp"):
                raise ValueError("invalid Nmap port")
            endpoint = f"{host_address}:{number}/{protocol}"
            findings.append({"file": file, "line": line, "rule": "network.open-port",
                             "severity": "note", "message": f"Observed open lab endpoint {endpoint}; confirm intended exposure. This is not exploit verification."})
    return {"findings": findings}


# An unfinished import exits nonzero, making the existing review tool incomplete.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--file", required=True)
    parser.add_argument("--line", type=int, required=True)
    args = parser.parse_args()
    print(json.dumps(convert(args.report.read_bytes(), args.file, args.line)))


if __name__ == "__main__":
    main()
