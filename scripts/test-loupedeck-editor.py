"""Opt-in Logi editor protocol regression check; never saves bindings or executes actions.

Requires PirateWave with integration enabled, the installed PirateWave plugin, and
`python -m pip install websocket-client`. Close action editor dialogs first.
"""
import collections
import json
import os
from pathlib import Path
import time
import websocket


def main():
    root = Path(os.environ['LOCALAPPDATA']) / 'Logi/LogiPluginService/Temp'
    port = (root / 'WebSocketPort.txt').read_text(encoding='utf-8').strip()
    connection = websocket.create_connection(
        f'ws://127.0.0.1:{port}/configui3', timeout=5, suppress_origin=True)
    request_id = 0
    checks = []

    def request(method, action, control, values):
        nonlocal request_id
        request_id += 1
        connection.send(json.dumps({
            'id': request_id, 'name': method,
            'parameters': {'actionName': action, 'controlName': control},
            'data': [{'name': key, 'value': value} for key, value in values.items()]
        }))
        events = []
        while True:
            response = json.loads(connection.recv())
            if response['id'] == request_id:
                assert not response['failed'], response.get('errorMessage')
                return response['data'], events
            if response['name'] == 'ProfileActionListboxItemsChanged' and response.get('data', {}).get('actionName') == action:
                events.append(response['data']['controlName'])

    def items(action, control, values):
        data, _ = request('GetProfileActionListboxItems', action, control, values)
        return data['items']

    def change(action, control, values, lists, expected):
        messages, events = request('ReportProfileActionControlValueChanged', action, control, values)
        assert set(events) == set(expected), (action, control, events)
        # Match the shipped renderer: exact dictionary keys, no case folding.
        for message in messages:
            key = message['controlName']
            assert key in values, ('Unknown renderer state key', key)
            if message['messageName'] == 'setValue':
                values[key] = message['messageValue']
        for key in events:
            assert key in values, ('Unknown renderer list cache key', key)
            lists[key] = items(action, key, values)

    try:
        for suffix in ['PirateWaveApplication', 'PirateWaveNode', 'PirateWaveEffect', 'PirateWaveSwitch',
                       'PirateWaveSound', 'PirateWaveRecording', 'PirateWaveMixer', 'PirateWaveEffectParameter',
                       'PirateWaveVstParameter', 'PirateWaveDuckingParameter',
                       'PirateWaveExtensionAction', 'PirateWaveExtensionParameter']:
            action = '$PirateWave___' + suffix
            values = {'node': '', 'target': '', 'action': '', 'sensitivity': '1'}
            lists = {'node': items(action, 'node', values), 'target': [], 'action': []}
            assert lists['node'], (suffix, 'Blank node list')
            first = lists['node'][0]
            assert first['name'] != '__piratewave_offline__', 'Run PirateWave and enable Loupedeck control'
            if first['name'] == '__piratewave_empty__':
                checks.append((suffix, 'No eligible targets'))
                continue
            values['node'] = first['name']
            change(action, 'node', values, lists, ['target', 'action'])
            assert lists['target'], (suffix, 'Visible target list stayed empty')
            values['target'] = lists['target'][0]['name']
            change(action, 'target', values, lists, ['action'])
            assert lists['action'], (suffix, 'Visible action/parameter list stayed empty')
            values['action'] = lists['action'][0]['name']
            change(action, 'action', values, lists, [])
            # Check the saved payload without creating a profile action.
            selector = json.loads(values['action'])
            assert selector['category'], suffix
            checks.append((suffix, len(lists['node']), len(lists['target']), len(lists['action'])))
        connection.settimeout(0.25)
        events = collections.Counter()
        start = time.monotonic()
        while time.monotonic() - start < 1.5:
            try:
                response = json.loads(connection.recv())
            except websocket.WebSocketTimeoutException:
                continue
            data = response.get('data')
            if isinstance(data, dict) and data.get('actionName', '').startswith('$PirateWave___'):
                events[response['name']] += 1
        assert events['ProfileActionListboxItemsChanged'] == 0, dict(events)
        print('PASS: selection messages, exact renderer cache keys, dependent lists and saved selectors:', checks)
        print('Idle PirateWave events in 1.5 seconds:', dict(events))
    finally:
        connection.close()


if __name__ == '__main__':
    main()
