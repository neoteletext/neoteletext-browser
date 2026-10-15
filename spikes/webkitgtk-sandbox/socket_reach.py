"""Tries to connect to every Unix socket path given as an argument, or on
standard input one per line, and prints the ones that accepted. Run inside a
confinement to see which local services it can still reach."""
import socket
import sys

reached = []
for path in sys.argv[1:] or [line.strip() for line in sys.stdin]:
    if not path.startswith("/"):
        continue
    for kind in (socket.SOCK_STREAM, socket.SOCK_SEQPACKET, socket.SOCK_DGRAM):
        client = socket.socket(socket.AF_UNIX, kind)
        try:
            client.connect(path)
            reached.append(path)
            break
        except OSError:
            pass
        finally:
            client.close()
print(f"{len(reached)} reachable: {' '.join(sorted(set(reached)))}")
