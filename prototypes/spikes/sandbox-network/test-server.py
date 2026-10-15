"""Proof A test server: logs every TCP connection's first line and every UDP
packet, so the sandbox's effect can be read from the log, not from the page."""
import socket, sys, threading, time

TCP_PORT, UDP_PORT, LOG = 47420, 47421, sys.argv[1]

def log(line):
    with open(LOG, "a") as handle:
        handle.write(f"{time.strftime('%H:%M:%S')} {line}\n")

def serve_tcp():
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("0.0.0.0", TCP_PORT))
    server.listen(64)
    while True:
        connection, _ = server.accept()
        try:
            connection.settimeout(2)
            first = connection.recv(4096).split(b"\r\n", 1)[0].decode("latin-1")
            log(f"TCP {first}")
            connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
        except OSError as error:
            log(f"TCP (no request: {error})")
        finally:
            connection.close()

def serve_udp():
    server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    server.bind(("0.0.0.0", UDP_PORT))
    while True:
        data, peer = server.recvfrom(2048)
        log(f"UDP {len(data)} bytes")

threading.Thread(target=serve_tcp, daemon=True).start()
threading.Thread(target=serve_udp, daemon=True).start()
while True:
    time.sleep(1)
