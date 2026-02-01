# FlowSDK Python Bindings

Python bindings for [FlowSDK](https://github.com/emqx/flowsdk).

## Installation

```bash
pip install flowsdk
```

## Usage

```python
import flowsdk

engine = flowsdk.MqttEngineFfi("client_id", 5)
engine.connect()
```
