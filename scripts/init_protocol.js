/**
 * One-time script to initialize the ProtocolConfig PDA on devnet.
 *
 * Usage:
 *   node scripts/init_protocol.js
 *
 * Prerequisites:
 *   - `solana config set --url devnet`
 *   - A funded keypair at ~/.config/solana/id.json
 */

const anchor = require("@coral-xyz/anchor");
const { PublicKey, SystemProgram } = require("@solana/web3.js");
const fs = require("fs");
const path = require("path");

async function main() {
  // --- Provider ---
  const connection = new anchor.web3.Connection(
    "https://api.devnet.solana.com",
    "confirmed"
  );

  const keypairPath = path.join(
    require("os").homedir(),
    ".config/solana/id.json"
  );
  const secretKey = Uint8Array.from(
    JSON.parse(fs.readFileSync(keypairPath, "utf-8"))
  );
  const admin = anchor.web3.Keypair.fromSecretKey(secretKey);

  const wallet = new anchor.Wallet(admin);
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "confirmed",
  });
  anchor.setProvider(provider);

  // --- Load IDL and program ---
  const idl = JSON.parse(
    fs.readFileSync(
      path.join(__dirname, "..", "target", "idl", "wager.json"),
      "utf-8"
    )
  );
  const programId = new PublicKey(
    "Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK"
  );
  const program = new anchor.Program(idl, provider);

  // --- Derive PDAs ---
  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    programId
  );

  console.log("Admin:      ", admin.publicKey.toBase58());
  console.log("Config PDA: ", configPda.toBase58());

  // --- Check if already initialized ---
  const configAccount = await connection.getAccountInfo(configPda);
  if (configAccount) {
    console.log("✅ ProtocolConfig is already initialized!");
    console.log("   Owner:", configAccount.owner.toBase58());
    console.log("   Data length:", configAccount.data.length);
  } else {
    console.log("⏳ Initializing ProtocolConfig...");

    // Use the admin's own address as the treasury for now
    const treasury = admin.publicKey;

    const tx = await program.methods
      .initializeProtocol(
        100, // default_fee_bps = 1% (100 basis points)
        new anchor.BN(7 * 24 * 60 * 60) // dispute_window_seconds = 7 days
      )
      .accounts({
        config: configPda,
        treasury: treasury,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    console.log("✅ ProtocolConfig initialized! Tx:", tx);
  }

  // --- Initialize the admin's own WagerRegistry (for testing) ---
  const [registryPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("registry"), admin.publicKey.toBytes()],
    programId
  );

  const registryAccount = await connection.getAccountInfo(registryPda);
  if (registryAccount) {
    console.log("✅ Admin WagerRegistry already initialized!");
  } else {
    console.log("⏳ Initializing admin WagerRegistry...");

    const tx2 = await program.methods
      .initializeRegistry()
      .accounts({
        registry: registryPda,
        authority: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    console.log("✅ Admin WagerRegistry initialized! Tx:", tx2);
  }

  console.log("\n🎉 Done! The protocol is ready for use on devnet.");
}

main().catch((err) => {
  console.error("Error:", err);
  process.exit(1);
});
