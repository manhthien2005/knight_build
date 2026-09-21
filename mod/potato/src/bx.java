/*
 * bx — the single wrapper every draw in the client goes through.
 * Verified sole funnel: scanning all 187 decompiled classes, only bx and ey
 * (minimap) touch javax.microedition.lcdui.Graphics directly.
 *
 * Patch: POTATO.countDraw() on each primitive that reaches Graphics, so the
 * per-second draw rate is measured rather than estimated. Drawing behaviour,
 * argument arithmetic and call order are unchanged.
 */
import javax.microedition.lcdui.Graphics;

public final class bx {
    public Graphics a;
    public static int b = 1;
    private int c;
    private int d;

    public final void a(aq aq2, int n2, int n3, int n4) {
        POTATO.countDraw();
        this.a.drawImage(aq2.a, n2 *= b, n3 *= b, n4);
    }

    public final void a(int n2, int n3, int n4, int n5) {
        POTATO.countDraw();
        n2 *= b;
        n3 *= b;
        n4 *= b;
        n5 *= b;
        int n6 = 0;
        while (n6 < b) {
            this.a.drawLine(n2 + n6, n3 + n6, n4 + n6, n5 + n6);
            if (n6 > 0) {
                this.a.drawLine(n2 + n6, n3, n4 + n6, n5);
                this.a.drawLine(n2, n3 + n6, n4, n5 + n6);
            }
            ++n6;
        }
    }

    public final void b(int n2, int n3, int n4, int n5) {
        POTATO.countDraw();
        n2 *= b;
        n3 *= b;
        n4 *= b;
        n5 *= b;
        int n6 = 0;
        while (n6 < b) {
            this.a.drawRect(n2 + n6, n3 + n6, n4 - (n6 << 1), n5 - (n6 << 1));
            ++n6;
        }
    }

    public final void a(aq aq2, int n2, int n3, int n4, int n5, int n6, int n7, int n8, int n9) {
        POTATO.countDraw();
        this.a.drawRegion(aq2.a, n2 *= b, n3 *= b, n4 *= b, n5 *= b, n6, n7 *= b, n8 *= b, n9);
    }

    public final void c(int n2, int n3, int n4, int n5) {
        POTATO.countDraw();
        this.a.fillRect(n2 *= b, n3 *= b, n4 *= b, n5 *= b);
    }

    public final void a(int n2, int n3, int n4, int n5, int n6, int n7) {
        POTATO.countDraw();
        this.a.fillRoundRect(n2 *= b, n3 *= b, n4 *= b, n5 *= b, n6 *= b, n7 *= b);
    }

    public final void b(int n2, int n3, int n4, int n5, int n6, int n7) {
        POTATO.countDraw();
        this.a.fillTriangle(n2 *= b, n3 *= b, n4 *= b, n5 *= b, n6 *= b, n7 *= b);
    }

    public final int a() {
        return this.a.getTranslateX() / b;
    }

    public final int b() {
        return this.a.getTranslateY() / b;
    }

    public final void d(int n2, int n3, int n4, int n5) {
        this.a.setClip(n2 *= b, n3 *= b, n4 *= b, n5 *= b);
    }

    public final void a(int n2) {
        this.a.setColor(n2);
    }

    public final void b(int n2) {
        this.a.setColor(0);
    }

    public final void a(int n2, int n3) {
        this.a.translate(n2 *= b, n3 *= b);
    }

    public final void a(int n2, int n3, int n4, int n5, int n6) {
        this.b(0, 0, fu.q.e * 24, n3, n6);
        this.b(0, n3, n2, fu.q.f * 24 - n3, n6);
        this.b(n2, n3 + n5, fu.q.e * 24 - n2, fu.q.f * 24 - (n3 + n5), n6);
        this.b(n2 + n4, n3, fu.q.e * 24 - (n2 + n4), n5, n6);
    }

    public final void b(int n2, int n3, int n4, int n5, int n6) {
        this.a(n6);
        this.c(n2, n3, n4, n5);
    }

    public final void e(int n2, int n3, int n4, int n5) {
        n2 *= b;
        n3 *= b;
        n4 *= b;
        n5 *= b;
        if (this.c != aq.a(cg.ax.a)) {
            this.c = aq.a(cg.ax.a);
        }
        if (this.d != aq.b(cg.ax.a)) {
            this.d = aq.b(cg.ax.a);
        }
        this.d(n2, n3, n4, n5);
        int n6 = n2 % this.c;
        while (n6 < n4 + this.c) {
            int n7 = n3 % this.d;
            while (n7 < n5 + this.d) {
                this.a(cg.ax, n2 + n4 - n6, n3 + n5 - n7, 20);
                n7 += this.d;
            }
            n6 += this.c;
        }
        this.d(-this.a.getTranslateX(), -this.a.getTranslateY(), fu.X, fu.Y);
    }
}
