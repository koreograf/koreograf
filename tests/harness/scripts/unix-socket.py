import os
import socket
import sys
import time

path = sys.argv[1]
mode = int(sys.argv[2], 8)
server = socket.socket(socket.AF_UNIX)
server.bind(path)
os.chmod(path, mode)
time.sleep(300)
