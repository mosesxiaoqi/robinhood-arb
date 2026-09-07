"""Offline checks for public T09 observations; no network or Rust quote code."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1]
data = root / 'tests/data/verified'
manifest = json.loads((data / 'manifest.json').read_text())
for name, entry in manifest['files'].items():
    assert hashlib.sha256((data / name).read_bytes()).hexdigest() == entry['sha256'], name
observations = json.loads((data / 'pons-observations.json').read_text())
contracts = json.loads((data / 'contracts.json').read_text())
network = json.loads((root / 'docs/verification/network-probe.json').read_text())
assert contracts[0]['runtime_code'] == network['requests'][6]['response']['result']
assert contracts[1]['runtime_code'] == next(
    x['response']['result'] for x in observations['requests']
    if x['request']['method'] == 'eth_getCode'
    and x['request']['params'][0].lower() == contracts[1]['address'].lower()
)
receipt = observations['requests'][4]['response']['result']
assert receipt['status'] == '0x1'
def words(prefix):
    log = next(x for x in receipt['logs'] if x['topics'][0].startswith(prefix))
    return [int(log['data'][i:i + 64], 16) for i in range(2, len(log['data']), 64)]
initial, liquidity, swap, fees = words('0xdd466e'), words('0xf208'), words('0x40e9'), words('0xc532')
price, active_liquidity, amount = initial[3], liquidity[2], (1 << 256) - swap[0]
q96 = 1 << 96
denominator = active_liquidity * q96 + amount * price
next_price = (active_liquidity * q96 * price + denominator - 1) // denominator
output = active_liquidity * (price - next_price) // q96
expected = json.loads((data / 'quote-expected.json').read_text())
assert next_price == swap[2] == int(expected['sqrt_price_after'])
assert output == swap[1] == int(expected['gross_output'])
assert output * 100 // 10000 == fees[1]
assert output * 200 // 10000 == fees[2]
assert output - fees[1] - fees[2] == int(expected['net_output'])
failed = json.loads((data / 'reverted-receipt.json').read_text())['receipt']
assert failed['status'] == '0x0' and not failed['logs']
print('T09: sample hashes, deployed code, swap arithmetic and separate fee rounding verified')

reverse = json.loads((data / 'reverse-observations.json').read_text())
expected = json.loads((data / 'reverse-expected.json').read_text())
logs = reverse['requests'][0]['response']['result']
swaps = [x for x in logs if x['topics'][0].startswith('0x40e9')]
def log_words(log):
    return [int(log['data'][i:i+64],16) for i in range(2,len(log['data']),64)]
index = next(i for i,log in enumerate(swaps) if log['transactionHash']==expected['transaction_hash'])
before, after = log_words(swaps[index-1]), log_words(swaps[index])
assert before[2] == int(expected['sqrt_price_before'])
assert before[3] == after[3] == int(expected['liquidity'])
# Between initialization and the sampled sell there are no further liquidity edits.
assert len([x for x in logs if x['topics'][0].startswith('0xf208')]) == 1
amount = (1<<256)-after[1]
next_price = before[2]+amount*q96//before[3]
output = ((before[3]*q96*(next_price-before[2]))//next_price)//before[2]
assert next_price == after[2] == int(expected['sqrt_price_after'])
assert output == after[0] == int(expected['gross_output'])
reverse_receipt = reverse['requests'][1]['response']['result']
assert reverse_receipt['status']=='0x1'
log = next(x for x in reverse_receipt['logs'] if x['logIndex']==swaps[index]['logIndex'])
for key in ['address','topics','data','blockHash','blockNumber','transactionHash','transactionIndex','logIndex','removed']:
    assert log[key]==swaps[index][key]
fee_log = next(x for x in reverse_receipt['logs'] if x['topics'][0].startswith('0xc532'))
fees = log_words(fee_log)
assert fees[1] == output*100//10000 == int(expected['hook_fee'])
assert fees[2] == output*200//10000 == int(expected['creator_tax'])
assert output-fees[1]-fees[2] == int(expected['net_output'])
print('T15: independent reverse swap state, output and fee rounding verified')
