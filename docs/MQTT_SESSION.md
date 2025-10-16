# Component: `mqtt_session`

MQTTv5 session implementation.

## Client Session

### Client Session State

1. Upstream publish packets

QoS 1 and QoS 2 messages which have been sent to the Server, but have not been completely acknowledged.

1. Downstream publish packets

QoS 2 messages which have been received from the Server, but have not been completely acknowledged.

1. Online knowledge

These knowledge are not preserved across Network Connections

- `send_quota` for flow control
  only for QoS > 0
  
1. Offline knowledge

- `session_expire_at` Time session will expire on the remote.


### Client Session functions

1. retry message after reconnect 

## Server Session

### Server Session State

1. Ongoing Sessions.

1. Client subscriptions.

1. Inflight downstream publish packets.

   QoS 1 and QoS 2 messages which have been sent to the Client, but have not been completely acknowledged.

1. Buffered publish packets.

   QoS 1 and QoS 2 messages pending transmission to the Client and OPTIONALLY QoS 0 messages pending transmission to the Client.
   
1. Unacked upstream publish packets.

   QoS 2 messages which have been received from the Client, but have not been completely acknowledged.

1. Session expire time 

   The session expire time if client is offline. 

