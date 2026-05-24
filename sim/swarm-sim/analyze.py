import polars as pl
import matplotlib.pyplot as plt

rfwpms = pl.read_parquet("/tmp/styx-rfwpms/piece_snapshots.parquet")
norfw = pl.read_parquet("/tmp/styx-norfw/piece_snapshots.parquet")

DANGER_THRESHOLD = 0

all_ticks = rfwpms.select("tick").unique()

rfwpms_danger = all_ticks.join(
    rfwpms.filter(pl.col("availability_count") == DANGER_THRESHOLD)
    .group_by("tick").agg(pl.col("piece_index").count().alias("lost_pieces")),
    on="tick", how="left"
).with_columns(pl.col("lost_pieces").fill_null(0)).sort("tick")

norfw_danger = all_ticks.join(
    norfw.filter(pl.col("availability_count") == DANGER_THRESHOLD)
    .group_by("tick").agg(pl.col("piece_index").count().alias("lost_pieces")),
    on="tick", how="left"
).with_columns(pl.col("lost_pieces").fill_null(0)).sort("tick")

fig, ax = plt.subplots(figsize=(11, 5))
fig.patch.set_facecolor("#0A0A0A")
ax.set_facecolor("#0A0A0A")

ax.plot(norfw_danger["tick"], norfw_danger["lost_pieces"],
        label="rarest-first (broken)", color="#FF4444", alpha=0.9, linewidth=1.5)
ax.plot(rfwpms_danger["tick"], rfwpms_danger["lost_pieces"],
        label="RFwPMS (fixed)", color="#00FF94", alpha=0.9, linewidth=1.5)

ax.set_xlabel("time (ticks)", color="#888888")
ax.set_ylabel("pieces lost from swarm (availability = 0)", color="#888888")
ax.tick_params(colors="#888888")
ax.spines[:].set_color("#333333")
ax.legend(facecolor="#111111", edgecolor="#333333", labelcolor="white")
ax.set_title("swarm stability: rarest-first vs RFwPMS", color="white", pad=15)

plt.tight_layout()
plt.savefig("sim/swarm-sim/rfwpms_comparison.png", dpi=150, facecolor="#0A0A0A")
print("saved rfwpms_comparison.png")
