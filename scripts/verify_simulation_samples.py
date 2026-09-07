"""Verify actual T22 RPC evidence offline; no fabricated success response."""
import hashlib
import json
from pathlib import Path
root = Path(__file__).resolve().parents[1]
data = root / 'tests/data/verified'
raw = json.loads((data / 'atomic-simulation.json').read_text())
expected = json.loads((data / 'atomic-expected.json').read_text())
artifact = json.loads((root / 'simulation/AtomicProbe.json').read_text())
assert hashlib.sha256((root / 'simulation/AtomicProbe.sol').read_bytes()).hexdigest() == artifact['source_sha256'] == expected['helper_source_sha256']
assert hashlib.sha256(bytes.fromhex(artifact['runtime_code'][2:])).hexdigest() == expected['helper_runtime_sha256']
def words(data):
    return [int(data[i:i+64],16) for i in range(2,len(data),64)]
base = expected['base_position']
assert raw['requests'][0]['response']['result']['hash'] == raw['requests'][-1]['response']['result']['hash'] == base['hash']
requests = [r for r in raw['requests'] if r['request']['method']=='eth_simulateV1']
good, bad = requests
for sample in requests:
    request = sample['request']
    assert int(request['params'][1],16) == base['number']
    block = request['params'][0]['blockStateCalls'][0]
    assert len(block['calls']) == 3
    assert block['stateOverrides']['0x000000000000000000000000000000000000beef']['code'] == artifact['runtime_code']
    assert set(block['stateOverrides']) == {'0x000000000000000000000000000000000000beef','0x000000000000000000000000000000000000cafe'}
    result = sample['response']['result'][0]
    assert result['parentHash'] == base['hash'] and int(result['number'],16) == base['number']+1
    assert result['calls'][0]['status'] == result['calls'][2]['status'] == '0x1'
success = good['response']['result'][0]['calls']
failed = bad['response']['result'][0]['calls']
assert success[1]['status']=='0x1' and failed[1]['status']=='0x0'
before, after = words(success[0]['returnData']), words(success[2]['returnData'])
middle, output = words(success[1]['returnData'])
assert middle == int(expected['middle_amount']) and output == int(expected['amount_out'])
amount = int(expected['amount_in'])
assert after[0] == before[0]-amount+output and after[1:4] == before[1:4]
assert after[4]!=before[4] and after[5]!=before[5]
assert words(failed[0]['returnData']) == words(failed[2]['returnData']) == before
swaps = [l for l in success[1]['logs'] if l['topics'][0].startswith('0x40e9')]
assert [l['topics'][1] for l in swaps] == expected['pool_ids']
first, second = map(lambda l:words(l['data']),swaps)
assert (1<<256)-first[0] == amount and (1<<256)-second[1] == middle and second[0] == output
fees = next(words(l['data']) for l in success[1]['logs'] if l['topics'][0].startswith('0xc532'))
assert middle == first[1]-fees[1]-fees[2]
assert not failed[1]['logs']
assert int(success[1]['gasUsed'],16) == expected['success_gas_used']
assert int(failed[1]['gasUsed'],16) == expected['reverted_gas_used']
a = good['request']['params'][0]['blockStateCalls'][0]['calls'][1]['data']
b = bad['request']['params'][0]['blockStateCalls'][0]['calls'][1]['data']
assert a[:-64] == b[:-64] and int(a[-64:],16)==0 and int(b[-64:],16)==1
assert output<amount  # Successful execution of this sample loses principal, before gas.
print('T22: actual two-pool execution, balance flow, base state and atomic rollback verified')
