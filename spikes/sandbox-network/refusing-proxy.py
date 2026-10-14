"""A live proxy that refuses every request with 403, and logs what it was
asked for. Unlike a dead proxy, it is reachable, so the Network
framework's failover to direct connections does not apply."""
import socket, sys, threading, time

PORT, LOG = 47422, sys.argv[1]

def log(line):
    with open(LOG, "a") as handle:
        handle.write(f"{time.strftime('%H:%M:%S')} {line}\n")

def handle(connection):
    try:
        connection.settimeout(2)
        first = connection.recv(4096).split(b"\r\n", 1)[0].decode("latin-1")
        log(f"PROXY asked: {first}")
        connection.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    except OSError as error:
        log(f"PROXY (no request: {error})")
    finally:
        connection.close()

server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", PORT))
server.listen(64)
while True:
    connection, _ = server.accept()
    threading.Thread(target=handle, args=(connection,), daemon=True).start()
