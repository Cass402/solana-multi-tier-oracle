// scripts/get-raydium-observation.ts
import { Connection, PublicKey } from "@solana/web3.js";

const RPC = process.env.RPC || "https://api.mainnet-beta.solana.com";
const POOL = process.argv[2];
if (!POOL) {
  console.error(
    "Usage: ts-node scripts/get-raydium-observation.ts <POOL_PUBKEY>"
  );
  process.exit(1);
}

(async () => {
  const conn = new Connection(RPC, "confirmed");
  const poolPk = new PublicKey(POOL);
  const info = await conn.getAccountInfo(poolPk);
  if (!info) throw new Error("Pool not found on mainnet");

  const data = Buffer.from(info.data);
  const observationOffset = 8 + 193; // discriminator + pool prefix
  const obsPk = new PublicKey(
    data.slice(observationOffset, observationOffset + 32)
  );

  console.log(obsPk.toBase58());
})();
