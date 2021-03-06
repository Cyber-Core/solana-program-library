import unittest
from base58 import b58decode
from solana_utils import *
from eth_tx_utils import  make_instruction_data_from_tx, pack
from sha3 import keccak_256
from hashlib import sha256

solana_url = os.environ.get("SOLANA_URL", "http://localhost:8899")
http_client = Client(solana_url)
CONTRACTS_DIR = os.environ.get("CONTRACTS_DIR", "evm_loader/")
evm_loader_id = os.environ.get("EVM_LOADER")
sysinstruct = "Sysvar1nstructions1111111111111111111111111"
keccakprog = "KeccakSecp256k11111111111111111111111111111"
sysvarclock = "SysvarC1ock11111111111111111111111111111111"
system = "11111111111111111111111111111111"


contract_name = "helloWorld.binary"
# "ERC20Wrapper.binary"


from construct import Bytes, Int8ul, Int64ul, Struct as cStruct
from solana._layouts.system_instructions import SYSTEM_INSTRUCTIONS_LAYOUT, InstructionType as SystemInstructionType

CREATE_ACCOUNT_LAYOUT = cStruct(
    "lamports" / Int64ul,
    "space" / Int64ul,
    "ether" / Bytes(20),
    "nonce" / Int8ul
)

def create_account_layout(lamports, space, ether, nonce):
    return bytes.fromhex("02000000")+CREATE_ACCOUNT_LAYOUT.build(dict(
        lamports=lamports,
        space=space,
        ether=ether,
        nonce=nonce
    ))

def write_layout(offset, data):
    return (bytes.fromhex("00000000")+
            offset.to_bytes(4, byteorder="little")+
            len(data).to_bytes(8, byteorder="little")+
            data)

def createAccountWithSeed(funding, base, seed, lamports, space, program):
    data = SYSTEM_INSTRUCTIONS_LAYOUT.build(
        dict(
            instruction_type = SystemInstructionType.CreateAccountWithSeed,
            args=dict(
                base=bytes(base),
                seed=dict(length=len(seed), chars=seed),
                lamports=lamports,
                space=space,
                program_id=bytes(program)
            )
        )
    )
    print("createAccountWithSeed", data.hex())
    created = accountWithSeed(base, seed, program) #PublicKey(sha256(bytes(base)+bytes(seed, 'utf8')+bytes(program)).digest())
    print("created", created)
    return TransactionInstruction(
        keys=[
            AccountMeta(pubkey=funding, is_signer=True, is_writable=True),
            AccountMeta(pubkey=created, is_signer=False, is_writable=True),
            AccountMeta(pubkey=base, is_signer=True, is_writable=False),
        ],
        program_id=system,
        data=data
    )


class DeployTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        wallet = WalletAccount(wallet_path())
        cls.loader = EvmLoader(wallet, evm_loader_id)
        cls.acc = wallet.get_acc()

        # Create ethereum account for user account
        cls.caller_ether = eth_keys.PrivateKey(cls.acc.secret_key()).public_key.to_canonical_address()
        (cls.caller, cls.caller_nonce) = cls.loader.ether2program(cls.caller_ether)

        if getBalance(cls.caller) == 0:
            print("Create caller account...")
            _ = cls.loader.createEtherAccount(cls.caller_ether)
            print("Done\n")

        print('Account:', cls.acc.public_key(), bytes(cls.acc.public_key()).hex())
        print("Caller:", cls.caller_ether.hex(), cls.caller_nonce, "->", cls.caller,
              "({})".format(bytes(PublicKey(cls.caller)).hex()))

    def executeTrxFromAccountData(self):
        # Create transaction holder account (if not exists)
        seed = "1236"
        holder = PublicKey(sha256(bytes(self.acc.public_key())+bytes(seed, 'utf8')+bytes(PublicKey(evm_loader_id))).digest())
        print("Holder", holder)
        if getBalance(holder) == 0:
            trx = Transaction()
            trx.add(createAccountWithSeed(self.acc.public_key(), self.acc.public_key(), "1236", 10**9, 128*1024, PublicKey(evm_loader_id)))
            result = http_client.send_transaction(trx, self.acc, opts=TxOpts(skip_confirmation=False))
            print(result)

        # Get nonce for caller
        trx_count = getTransactionCount(http_client, self.caller)

        # Create contract address from (caller, nonce)
        contract_eth = keccak_256(pack([self.caller_ether, trx_count or None])).digest()[-20:]
        (contract_sol, contract_nonce) = self.loader.ether2program(contract_eth)
        (code_sol, code_nonce) = self.loader.ether2seed(contract_eth)
        print("contract_eth", contract_eth.hex())
        print("contract_sol", contract_sol, contract_nonce)
        print("code_sol", code_sol)

        # Read content of solidity contract
        with open(CONTRACTS_DIR+contract_name, "br") as f:
            content = f.read()

        # Build deploy transaction
        tx = {
            'to': None,
            'value': 0,
            'gas': 1,
            'gasPrice': 1,
            'nonce': trx_count,
            'data': content,
            'chainId': 111
        }
        (from_addr, sign, msg) = make_instruction_data_from_tx(tx, self.acc.secret_key())
        msg = sign + len(msg).to_bytes(8, byteorder="little") + msg
        #print("msg", msg.hex())

        # Write transaction to transaction holder account
        offset = 0
        receipts = []
        rest = msg
        while len(rest):
            (part, rest) = (rest[:1000], rest[1000:])
            trx = Transaction()
            trx.add(TransactionInstruction(program_id=evm_loader_id,
                data=write_layout(offset, part),
                keys=[
                    AccountMeta(pubkey=holder, is_signer=False, is_writable=True),
                    AccountMeta(pubkey=self.acc.public_key(), is_signer=True, is_writable=False),
                ]))
            receipts.append(http_client.send_transaction(trx, self.acc)["result"])
            offset += len(part)
        print("receipts", receipts)
        for rcpt in receipts:
            confirm_transaction(http_client, rcpt)
            print("confirmed:", rcpt)

        base = self.acc.public_key()
        seed = b58encode(contract_eth).decode('utf8')
        # Execute deploy transaction
        trx = Transaction()

        trx.add(createAccountWithSeed(base, base, seed, 10**9, 1+32+4+len(msg)+2048, PublicKey(evm_loader_id)))

        trx.add(TransactionInstruction(program_id=evm_loader_id,
            #data=create_account_layout(10**9, len(msg)+2048, contract_eth, contract_nonce),
            data=bytes.fromhex('02000000')+CREATE_ACCOUNT_LAYOUT.build(dict(
                lamports=10**9,
                space=0,
                ether=contract_eth,
                nonce=contract_nonce)),
            keys=[
                AccountMeta(pubkey=self.acc.public_key(), is_signer=True, is_writable=False),
                AccountMeta(pubkey=contract_sol, is_signer=False, is_writable=True),
                AccountMeta(pubkey=code_sol, is_signer=False, is_writable=True),
                AccountMeta(pubkey=system, is_signer=False, is_writable=False),
            ]))

        result = http_client.send_transaction(trx, self.acc,
                        opts=TxOpts(skip_confirmation=False, preflight_commitment="root"))["result"]
        print("result executeTrxFromAccountData:", result)

        return (holder, contract_sol, code_sol)


    def sol_instr_11_partial_call(self, storage_account, step_count, holder, contract_sol, code_sol):
        return TransactionInstruction(program_id=self.loader.loader_id,
                                   data=bytearray.fromhex("0b") + step_count.to_bytes(8, byteorder='little'),
                                   # data=bytearray.fromhex("0b"),
                                   keys=[
                                       AccountMeta(pubkey=holder, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=storage_account, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=contract_sol, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=code_sol, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=self.caller, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=self.loader.loader_id, is_signer=False, is_writable=False),
                                       AccountMeta(pubkey=PublicKey(sysvarclock), is_signer=False, is_writable=False),
                                   ])

    def sol_instr_10_continue(self, storage_account, step_count, contract_sol, code_sol):
        return TransactionInstruction(program_id=self.loader.loader_id,
                                   data=bytearray.fromhex("0A") + step_count.to_bytes(8, byteorder='little'),
                                   keys=[
                                       AccountMeta(pubkey=storage_account, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=contract_sol, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=code_sol, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=self.caller, is_signer=False, is_writable=True),
                                       AccountMeta(pubkey=self.loader.loader_id, is_signer=False, is_writable=False),
                                       AccountMeta(pubkey=PublicKey(sysvarclock), is_signer=False, is_writable=False),
                                   ])


    def create_storage_account(self, seed):
        storage = PublicKey(sha256(bytes(self.acc.public_key()) + bytes(seed, 'utf8') + bytes(PublicKey(evm_loader_id))).digest())
        print("Storage", storage)

        if getBalance(storage) == 0:
            trx = Transaction()
            trx.add(createAccountWithSeed(self.acc.public_key(), self.acc.public_key(), seed, 10**9, 128*1024, PublicKey(evm_loader_id)))
            http_client.send_transaction(trx, self.acc, opts=TxOpts(skip_confirmation=False))

        return storage

    def call_partial_signed(self, holder, contract_sol, code_sol):
        storage = self.create_storage_account("001122334")

        print("Begin")
        trx = Transaction()
        trx.add(self.sol_instr_11_partial_call(storage, 50, holder, contract_sol, code_sol))
        result = http_client.send_transaction(trx, self.acc, opts=TxOpts(skip_confirmation=False, preflight_commitment="root"))["result"]

        while (True):
            print("Continue")
            trx = Transaction()
            trx.add(self.sol_instr_10_continue(storage, 50, contract_sol, code_sol))
            result = http_client.send_transaction(trx, self.acc, opts=TxOpts(skip_confirmation=False, preflight_commitment="root"))["result"]

            if (result['meta']['innerInstructions'] and result['meta']['innerInstructions'][0]['instructions']):
                data = b58decode(result['meta']['innerInstructions'][0]['instructions'][-1]['data'])
                if (data[0] == 6):
                    return result

    def test_executeTrxFromAccountDataIterative(self):
        (holder, contract_sol, code_sol) = self.executeTrxFromAccountData()

        result = self.call_partial_signed(holder, contract_sol, code_sol)
        print("result", result)


    def test_executeTrxFromAccountData(self):
        (holder, contract_sol, code_sol) = self.executeTrxFromAccountData()
        print("test_executeTrxFromAccountData")

        trx = Transaction()
        trx.add(TransactionInstruction(program_id=evm_loader_id,
            data=bytes.fromhex('08'),
            keys=[
                AccountMeta(pubkey=holder, is_signer=False, is_writable=True),
                AccountMeta(pubkey=contract_sol, is_signer=False, is_writable=True),
                AccountMeta(pubkey=code_sol, is_signer=False, is_writable=True),
                AccountMeta(pubkey=self.caller, is_signer=False, is_writable=True),
                AccountMeta(pubkey=evm_loader_id, is_signer=False, is_writable=False),
                AccountMeta(pubkey=PublicKey(sysvarclock), is_signer=False, is_writable=False),
            ]))
        result = http_client.send_transaction(trx, self.acc,
                        opts=TxOpts(skip_confirmation=False, preflight_commitment="root"))["result"]
        print("result", result)

if __name__ == '__main__':
    unittest.main()
