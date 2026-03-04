/**
 * One-time script to migrate ProtocolConfig to add USDC mint.
 *
 * Usage:
 *   node scripts/migrate_config.js
 */

const anchor = require("@coral-xyz/anchor");
const { PublicKey, SystemProgram } = require("@solana/web3.js");
const fs = require("fs");
const path = require("path");

// USDC Mint addresses (6 decimals)
const DEVNET_USDC_MINT = new PublicKey("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
const MAINNET_USDC_MINT = new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

async function main() {
  // --- Provider ---
  const rpcUrl = process.env.SOLANA_RPC_URL || "https://api.devnet.solana.com";
  const isMainnet = rpcUrl.includes("mainnet");
  const usdcMint = isMainnet ? MAINNET_USDC_MINT : DEVNET_USDC_MINT;
  
  const connection = new anchor.web3.Connection(rpcUrl, "confirmed");

  const keypairPath = path.join(require("os").homedir(), ".config/solana/id.json");
  const secretKey = Uint8Array.from(JSON.parse(fs.readFileSync(keypairPath, "utf-8")));
  const admin = anchor.web3.Keypair.fromSecretKey(secretKey);

  const wallet = new anchor.Wallet(admin);
  const provider = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });
  anchor.setProvider(provider);

  // --- Load IDL and program ---
  const idl = JSON.parse(
    fs.readFileSync(path.join(__dirname, "..", "target", "idl", "wager.json"), "utf-8")
  );
  const programId = new PublicKey("Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK");
  const program = new anchor.Program(idl, provider);

  // --- Derive config PDA ---
  const [configPda] = PublicKey.findProgramAddressSync([Buffer.from("config")], programId);

  console.log("Admin:      ", admin.publicKey.toBase58());
  console.log("Config PDA: ", configPda.toBase58());
  console.log("USDC Mint:  ", usdcMint.toBase58());
  console.log("Network:    ", isMainnet ? "mainnet" : "devnet");

  // --- Check current config ---
  const configAccount = await connection.getAccountInfo(configPda);
  if (!configAccount) {
    console.log("❌ ProtocolConfig not found. Run init_protocol.js first.");
    process.exit(1);
  }

  console.log("Current config data length:", configAccount.data.length);
  console.log("Expected config length:", 8 + 1 + 32 + 32 + 2 + 8 + 1 + 32); // 116 bytes

  // --- Run migration ---
  console.log("⏳ Running config migration...");

  try {
    const tx = await program.methods
      .migrateConfig()
      .accounts({
        config: configPda,
        usdcMint: usdcMint,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    console.log("✅ Config migrated! Tx:", tx);
  } catch (err) {
    console.error("Migration failed:", err.message);
    if (err.logs) {
      console.error("Logs:", err.logs);
    }
  }

  console.log("\n🎉 Done!");
}

main().catch((err) => {
  console.error("Error:", err);
  process.exit(1);
});
