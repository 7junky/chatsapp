# chatsapp

A simple application where users can talk to each other in rooms.

## Usage

Run `make` to start Redis and `cargo run` to start the server.
You can then connect to the server using `nc` or `telnet` eg `nc localhost 8000`

```
>help
Commands:
>help              - Display commands
>exit              - Close connection
>list              - List rooms
>me                - Your user info
>set-username name - Set username
>create-room room  - Create room
>join-room room    - Join room
```

## Implementation

Rooms and messages are persisted using Redis. Every time the server starts, rooms are fetched from Redis and broker tasks are spawned for each of them.
Each broker task will have an mpsc `Sender` stored in a map, which can be fetched any time someone intends on joining a room. Here are the events it expects:

* `BrokerEvent::JoinRoom` - The broker keeps a map of who is currently connected to the room. When someone joins, a channel is created and they're inserted to
the map with their `Sender`. Then a task is spawned with the `Receiver` and users `TcpStream`, which waits for messages and writes them to the users.

* `BrokerEvent::LeaveRoom` - This removes a user from the brokers users map. This causes the `Sender` to get dropped, which then results in the receiver task closing.

* `BrokerEvent::Message` - This sends a message to all users connected to the room.
