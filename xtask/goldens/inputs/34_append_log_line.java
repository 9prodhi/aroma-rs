import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.IOException;

class AppendLogLine {
    void appendLog(String path, String level, String message) throws IOException {
        BufferedWriter writer = new BufferedWriter(new FileWriter(path, true));
        String stamp = String.valueOf(System.currentTimeMillis());
        writer.write(stamp + " " + level + " " + message);
        writer.newLine();
        writer.flush();
        writer.close();
    }
}
