import 'dart:convert';
import 'package:codux_flutter/models/remote_models.dart';
import 'package:codux_flutter/services/remote_protocol_service.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('envelope request correlation survives parsing and device scoping', () {
    final envelope = RelayEnvelope.fromJson({
      'type': 'ai.session.result',
      'id': 'message-id',
      'requestId': 'restore-request',
      'payload': {'op': 'restore'},
    });
    final scoped = envelope.copyWith(deviceId: 'phone', seq: 7);
    expect(scoped.requestId, 'restore-request');
    expect(scoped.id, 'message-id');
    expect(scoped.toJson()['requestId'], 'restore-request');
    expect(scoped.toJson()['id'], 'message-id');
    expect(scoped.copyWith(requestId: 'next').requestId, 'next');
  });

  test('stored device reads iroh transport candidates', () {
    final device = StoredDevice.fromJson({
      'server': 'https://relay.example',
      'hostId': 'host-1',
      'deviceId': 'device-1',
      'token': '',
      'name': 'Phone',
      'transports': [irohCandidate(url: 'https://relay.example')],
    });

    expect(
      remotePreferredTransportKind(device.transports, pairing: false),
      RemoteTransportKind.iroh,
    );
    expect(
      device.transportByKind(RemoteTransportKind.iroh)?.url,
      'https://relay.example',
    );
    expect(device.transportByKind(RemoteTransportKind.iroh)?.nodeId, 'node-1');
  });

  test(
    'embedded iroh ticket pairing payload parses without relay ticket fetch',
    () async {
      final qr = embeddedPairingQr({
        'code': '205503D6',
        'secret': 'pairing-secret',
        'pairingId': 'pair-1',
        'transports': [
          irohCandidate(
            ticket: 'endpointabc',
            relayAuthentication: 'relay-token',
          ),
        ],
      });

      final payload = await parsePairingPayload(qr);
      final transport = payload.transportByKind(RemoteTransportKind.iroh);
      expect(payload.server, '');
      expect(payload.hostId, isNull);
      expect(payload.hostName, isNull);
      expect(transport?.ticket, 'endpointabc');
      expect(transport?.relayAuthentication, 'relay-token');
      expect(transport?.nodeId, isEmpty);
      expect(transport?.relayUrl, isEmpty);
    },
  );

  test(
    'embedded iroh token payload and confirmation use transport candidates',
    () async {
      final qr = embeddedPairingQr({
        'code': '205503D6',
        'secret': 'pairing-secret',
        'pairingId': 'pair-1',
        'transports': [irohCandidate(ticket: 'endpoint-ticket')],
      });

      final payload = await parsePairingPayload(qr);
      expect(
        remotePreferredTransportKind(payload.transports, pairing: true),
        RemoteTransportKind.iroh,
      );
      expect(payload.hostId, isNull);
      expect(payload.pairingId, 'pair-1');
      expect(
        payload.transportByKind(RemoteTransportKind.iroh)?.ticket,
        'endpoint-ticket',
      );

      final request = pairingRequestEnvelope(payload, 'Phone');
      expect(request.type, 'pairing.request');
      expect((request.payload as Map)['pairingId'], 'pair-1');
      expect((request.payload as Map)['deviceName'], 'Phone');
      expect((request.payload as Map)['deviceId'], payload.deviceId);

      final confirmed = confirmedDevice(
        payload: payload,
        name: 'Phone',
        confirmed: RelayEnvelope(
          type: 'pairing.confirmed',
          payload: {
            'hostId': 'host-1',
            'deviceId': 'device-1',
            'token': '',
            'hostName': 'Mac',
            'transports': [irohCandidate(url: 'https://relay.example')],
          },
        ),
      );
      expect(
        remotePreferredTransportKind(confirmed.transports, pairing: false),
        RemoteTransportKind.iroh,
      );
      expect(confirmed.server, 'https://relay.example');
      expect(confirmed.deviceId, 'device-1');
      expect(confirmed.toJson()['transports'], [
        irohCandidate(url: 'https://relay.example'),
      ]);
    },
  );

  test('pasted bare token parses like a scanned qr payload', () async {
    final manual = embeddedPairingToken({
      'code': '123456',
      'secret': 'pairing-secret',
      'pairingId': 'pair-1',
      'transports': [irohCandidate(ticket: 'endpoint-ticket')],
    });

    final payload = await parsePairingPayload(manual);
    expect(payload.server, '');
    expect(payload.code, '123456');
    expect(payload.hostId, isNull);
    expect(payload.pairingId, 'pair-1');
    expect(
      payload.transportByKind(RemoteTransportKind.iroh)?.ticket,
      'endpoint-ticket',
    );
  });

  test('pairing payload rejects missing iroh transport', () async {
    final qr = embeddedPairingQr({
      'code': '205503D6',
      'secret': 'pairing-secret',
      'pairingId': 'pair-1',
      'transports': const [],
    });

    expect(() => parsePairingPayload(qr), throwsException);
  });

  test('pairing payload reports missing required fields', () async {
    final qr = embeddedPairingQr({'code': '205503D6'});

    expect(
      () => parsePairingPayload(qr),
      throwsA(
        isA<Exception>()
            .having((error) => error.toString(), 'message', contains('secret'))
            .having(
              (error) => error.toString(),
              'message',
              contains('pairingId'),
            )
            .having(
              (error) => error.toString(),
              'message',
              contains('transports.iroh'),
            ),
      ),
    );
  });

  test('confirmation rejects incomplete device credentials', () async {
    final payload = await parsePairingPayload(
      embeddedPairingQr({
        'code': '205503D6',
        'secret': 'pairing-secret',
        'pairingId': 'pair-1',
        'transports': [irohCandidate(ticket: 'endpoint-ticket')],
      }),
    );
    expect(
      () => confirmedDevice(
        payload: payload,
        name: 'Phone',
        confirmed: const RelayEnvelope(
          type: 'pairing.confirmed',
          payload: {'hostId': 'host-1'},
        ),
      ),
      throwsA(
        isA<Exception>().having(
          (error) => error.toString(),
          'message',
          contains('Pairing confirmed without device credentials'),
        ),
      ),
    );
  });

  test('pairing payload accepts iroh ticket without host id', () async {
    final qr = embeddedPairingQr({
      'code': '205503D6',
      'secret': 'pairing-secret',
      'pairingId': 'pair-1',
      'transports': [irohCandidate(ticket: 'endpoint-ticket')],
    });

    final payload = await parsePairingPayload(qr);
    expect(payload.hostId, isNull);
    expect(
      payload.transportByKind(RemoteTransportKind.iroh)?.ticket,
      'endpoint-ticket',
    );
  });

  test(
    'embedded payload keeps its iroh candidate url as connection authority',
    () async {
      final qr = embeddedPairingQr({
        'code': '205503D6',
        'secret': 'pairing-secret',
        'pairingId': 'pair-1',
        'transports': [irohCandidate(ticket: 'endpoint-ticket')],
      });

      final payload = await parsePairingPayload(qr);

      expect(payload.server, '');
      expect(
        payload.transportByKind(RemoteTransportKind.iroh)?.ticket,
        'endpoint-ticket',
      );

      final confirmed = confirmedDevice(
        payload: payload,
        name: 'Phone',
        confirmed: RelayEnvelope(
          type: 'pairing.confirmed',
          payload: {
            'hostId': 'host-1',
            'deviceId': 'device-1',
            'token': '',
            'hostName': 'Mac',
            'transports': [
              irohCandidate(url: 'https://confirmed-relay.example'),
            ],
          },
        ),
      );

      expect(confirmed.server, 'https://confirmed-relay.example');
      expect(
        confirmed.transportByKind(RemoteTransportKind.iroh)?.url,
        'https://confirmed-relay.example',
      );
    },
  );

  test(
    'confirmed device strips ephemeral iroh ticket before persistence',
    () async {
      final payload = await parsePairingPayload(
        embeddedPairingQr({
          'code': '205503D6',
          'secret': 'pairing-secret',
          'pairingId': 'pair-1',
          'transports': [irohCandidate(ticket: 'endpoint-ticket')],
        }),
      );

      final confirmed = confirmedDevice(
        payload: payload,
        name: 'Phone',
        confirmed: const RelayEnvelope(
          type: 'pairing.confirmed',
          payload: {
            'hostId': 'host-1',
            'deviceId': 'device-1',
            'token': '',
            'transports': [
              {
                'kind': RemoteTransportKind.iroh,
                'url': 'https://relay.example',
                'nodeId': 'node-1',
                'relayUrl': 'https://relay.example',
              },
            ],
          },
        ),
      );

      expect(confirmed.transports.single.ticket, isEmpty);
      expect(confirmed.toJson()['transports'], [
        irohCandidate(url: 'https://relay.example'),
      ]);
    },
  );
}

Map<String, dynamic> irohCandidate({
  String url = '',
  String ticket = '',
  String relayAuthentication = '',
}) => {
  'kind': RemoteTransportKind.iroh,
  if (url.isNotEmpty) 'url': url,
  if (url.isNotEmpty) 'nodeId': 'node-1',
  if (url.isNotEmpty) 'relayUrl': 'https://relay.example',
  if (ticket.isNotEmpty) 'ticket': ticket,
  if (relayAuthentication.isNotEmpty)
    'relayAuthentication': relayAuthentication,
};

String embeddedPairingQr(Map<String, dynamic> payload) {
  final encoded = base64Url.encode(utf8.encode(jsonEncode(payload)));
  return Uri(
    scheme: 'codux',
    host: 'pair',
    queryParameters: {'payload': encoded},
  ).toString();
}

String embeddedPairingToken(Map<String, dynamic> payload) {
  return base64Url.encode(utf8.encode(jsonEncode(payload)));
}
