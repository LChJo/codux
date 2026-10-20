import '../models/remote_models.dart';
import 'remote_transport.dart';

typedef RemoteSendErrorHandler = void Function(Object error);
typedef RemoteSendResultHandler =
    void Function(RelayEnvelope message, RemoteEnvelopeSendResult result);

enum RemoteEnvelopeSendResult { delivered, droppedWhileDisconnected, rejected }

class RemoteEnvelopeSendQueue {
  int _seq = 0;
  int _generation = 0;
  Future<void> _chain = Future<void>.value();

  void reset({int? seed}) {
    _generation++;
    _seq = seed ?? 0;
    _chain = Future<void>.value();
  }

  Future<void> send({
    required RelayEnvelope message,
    required RemoteTransport transport,
    required bool Function() connected,
    bool Function()? stillValid,
    StoredDevice? activeDevice,
    bool terminalStream = false,
    RemoteSendErrorHandler? onError,
    RemoteSendResultHandler? onResult,
  }) {
    final seq = activeDevice == null ? null : ++_seq;
    final generation = _generation;
    bool valid() =>
        generation == _generation &&
        connected() &&
        (stillValid?.call() ?? true);
    final previous = _chain.catchError((_) {});
    final task = previous
        .then((_) async {
          if (!valid()) {
            onResult?.call(
              message,
              RemoteEnvelopeSendResult.droppedWhileDisconnected,
            );
            return;
          }
          final outgoing = _attachDeviceIdentity(message, activeDevice, seq);
          final envelope = outgoing.toJson();
          late final bool sent;
          try {
            sent = terminalStream
                ? await transport.sendTerminal(envelope)
                : await transport.send(envelope);
          } catch (error) {
            if (!valid()) {
              onResult?.call(
                message,
                RemoteEnvelopeSendResult.droppedWhileDisconnected,
              );
              return;
            }
            onResult?.call(message, RemoteEnvelopeSendResult.rejected);
            onError?.call(error);
            return;
          }
          onResult?.call(
            message,
            !valid()
                ? RemoteEnvelopeSendResult.droppedWhileDisconnected
                : sent
                ? RemoteEnvelopeSendResult.delivered
                : RemoteEnvelopeSendResult.rejected,
          );
        })
        .catchError((Object error) {
          onError?.call(error);
        });
    _chain = task;
    return task;
  }

  RelayEnvelope _attachDeviceIdentity(
    RelayEnvelope message,
    StoredDevice? activeDevice,
    int? seq,
  ) {
    if (activeDevice == null) {
      return seq == null ? message : message.copyWith(seq: seq);
    }
    return message.copyWith(
      hostId: activeDevice.hostId,
      deviceId: activeDevice.deviceId,
      seq: seq,
    );
  }
}
