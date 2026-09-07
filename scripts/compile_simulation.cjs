// Optional verification build; the Rust runtime consumes the pinned artifact.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '..');
const solc = require(process.argv[2] || path.join(root, 'simulation/node_modules/solc'));
if (!solc.version().startsWith('0.8.35+commit.47b9dedd')) throw new Error('unexpected compiler version');
const source = fs.readFileSync(path.join(root, 'simulation/AtomicProbe.sol'), 'utf8');
const settings = { optimizer: { enabled: true, runs: 200 }, viaIR: true, evmVersion: 'cancun', outputSelection: { '*': { '*': ['abi', 'evm.deployedBytecode.object'] } } };
const output = JSON.parse(solc.compile(JSON.stringify({ language: 'Solidity', sources: { 'AtomicProbe.sol': { content: source } }, settings })));
for (const issue of output.errors || []) { if (issue.severity === 'error') throw new Error(issue.formattedMessage); }
const contract = output.contracts['AtomicProbe.sol'].AtomicProbe;
const artifact = { compiler: solc.version(), settings, source_sha256: crypto.createHash('sha256').update(source).digest('hex'), abi: contract.abi, runtime_code: `0x${contract.evm.deployedBytecode.object}` };
fs.writeFileSync(path.join(root, 'simulation/AtomicProbe.json'), JSON.stringify(artifact, null, 2) + '\n');
console.log(`AtomicProbe runtime: ${contract.evm.deployedBytecode.object.length / 2} bytes`);
