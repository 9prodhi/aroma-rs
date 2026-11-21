import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;

class CopyFileBytes {
    long copy(String from, String to) throws IOException {
        FileInputStream in = new FileInputStream(from);
        FileOutputStream out = new FileOutputStream(to);
        byte[] buffer = new byte[8192];
        long total = 0;
        int read = in.read(buffer);
        while (read > 0) {
            out.write(buffer, 0, read);
            total += read;
            read = in.read(buffer);
        }
        out.flush();
        in.close();
        out.close();
        return total;
    }
}
